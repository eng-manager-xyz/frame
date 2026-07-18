use std::collections::BTreeMap;

use frame_domain::{
    ByteSize, ChecksumSha256, ContentType, ObjectKey, ObjectRole, ObjectVersion, TenantId,
    UploadContract, UploadState, VideoId,
};
use frame_ports::{
    AdvancedObjectStore, CompletedPart, MultipartUpload, ObjectMetadata, PartNumber, PutOptions,
    UploadedPart, WriteCondition,
};

use crate::ApplicationError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPartReceipt {
    pub offset: ByteSize,
    pub uploaded: UploadedPart,
    checksum: Option<ChecksumSha256>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginUpload {
    pub tenant_id: TenantId,
    pub video_id: VideoId,
    pub version: ObjectVersion,
    pub file_name: String,
    pub expected_size: ByteSize,
    pub content_type: ContentType,
    pub checksum: Option<ChecksumSha256>,
}

#[derive(Debug, Clone)]
pub struct UploadSession {
    pub contract: UploadContract,
    pub object_key: ObjectKey,
    backend: MultipartUpload,
    parts: BTreeMap<PartNumber, UploadPartReceipt>,
    completed: Option<ObjectMetadata>,
}

/// Coordinates the domain upload state machine with a multipart object-store adapter.
pub struct MultipartUploadCoordinator<'a> {
    store: &'a dyn AdvancedObjectStore,
}

impl<'a> MultipartUploadCoordinator<'a> {
    #[must_use]
    pub const fn new(store: &'a dyn AdvancedObjectStore) -> Self {
        Self { store }
    }

    pub async fn begin(&self, request: BeginUpload) -> Result<UploadSession, ApplicationError> {
        if request.expected_size.get() == 0 {
            return Err(ApplicationError::Invalid);
        }
        let object_key = ObjectKey::for_video(
            request.tenant_id,
            request.video_id,
            ObjectRole::Source,
            request.version,
            &request.file_name,
        )
        .map_err(|_| ApplicationError::Invalid)?;
        let backend = self
            .store
            .begin_multipart(
                &object_key,
                PutOptions {
                    content_type: request.content_type,
                    checksum_sha256: request.checksum,
                    condition: WriteCondition::IfAbsent,
                },
            )
            .await?;
        let mut contract =
            UploadContract::new(request.tenant_id, request.video_id, request.expected_size);
        contract.begin().map_err(|_| ApplicationError::Invalid)?;
        Ok(UploadSession {
            contract,
            object_key,
            backend,
            parts: BTreeMap::new(),
            completed: None,
        })
    }

    pub async fn upload_part(
        &self,
        tenant_id: TenantId,
        session: &mut UploadSession,
        part_number: PartNumber,
        offset: ByteSize,
        bytes: Vec<u8>,
        checksum: Option<ChecksumSha256>,
    ) -> Result<UploadPartReceipt, ApplicationError> {
        authorize(tenant_id, session)?;
        let size =
            ByteSize::new(u64::try_from(bytes.len()).map_err(|_| ApplicationError::Invalid)?)
                .map_err(|_| ApplicationError::Invalid)?;
        if size.get() == 0 {
            return Err(ApplicationError::Invalid);
        }
        if let Some(previous) = session.parts.get(&part_number) {
            return if previous.offset == offset
                && previous.uploaded.size == size
                && previous.checksum == checksum
            {
                Ok(previous.clone())
            } else {
                Err(ApplicationError::Conflict)
            };
        }
        if offset != session.contract.received_size {
            return Err(ApplicationError::Conflict);
        }
        let uploaded = self
            .store
            .upload_part(session.backend.id, part_number, bytes, checksum.clone())
            .await?;
        session
            .contract
            .record_chunk(offset, uploaded.size)
            .map_err(|_| ApplicationError::Invalid)?;
        let receipt = UploadPartReceipt {
            offset,
            uploaded,
            checksum,
        };
        session.parts.insert(part_number, receipt.clone());
        Ok(receipt)
    }

    pub async fn finalize(
        &self,
        tenant_id: TenantId,
        session: &mut UploadSession,
    ) -> Result<ObjectMetadata, ApplicationError> {
        authorize(tenant_id, session)?;
        if let Some(metadata) = &session.completed {
            return Ok(metadata.clone());
        }
        session
            .contract
            .begin_finalizing()
            .map_err(|_| ApplicationError::Invalid)?;
        let parts = session
            .parts
            .values()
            .map(|part| CompletedPart {
                part_number: part.uploaded.part_number,
                etag: part.uploaded.etag.clone(),
            })
            .collect::<Vec<_>>();
        let metadata = self
            .store
            .complete_multipart(session.backend.id, &parts)
            .await?;
        if metadata.size != session.contract.expected_size {
            return Err(ApplicationError::Conflict);
        }
        session
            .contract
            .complete()
            .map_err(|_| ApplicationError::Internal)?;
        session.completed = Some(metadata.clone());
        Ok(metadata)
    }

    pub async fn abort(
        &self,
        tenant_id: TenantId,
        session: &mut UploadSession,
    ) -> Result<(), ApplicationError> {
        authorize(tenant_id, session)?;
        if session.contract.state == UploadState::Complete {
            return Err(ApplicationError::Conflict);
        }
        self.store.abort_multipart(session.backend.id).await?;
        session
            .contract
            .abort()
            .map_err(|_| ApplicationError::Conflict)
    }
}

fn authorize(tenant_id: TenantId, session: &UploadSession) -> Result<(), ApplicationError> {
    if session.contract.tenant_id == tenant_id && session.object_key.belongs_to_tenant(tenant_id) {
        Ok(())
    } else {
        // Deliberately indistinguishable from an unknown upload.
        Err(ApplicationError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use frame_domain::{
        ByteSize, ChecksumSha256, ContentType, ObjectVersion, TenantId, UploadState, VideoId,
    };
    use frame_ports::{AdvancedObjectStore, MemoryObjectStore, PartNumber};

    use super::*;

    fn size(value: u64) -> ByteSize {
        ByteSize::new(value).expect("valid size")
    }

    fn checksum(value: char) -> ChecksumSha256 {
        ChecksumSha256::parse(value.to_string().repeat(64)).expect("valid checksum")
    }

    #[tokio::test]
    async fn multipart_upload_resumes_replays_and_finalizes_once() {
        let store = MemoryObjectStore::default();
        let coordinator = MultipartUploadCoordinator::new(&store);
        let tenant = TenantId::new();
        let mut session = coordinator
            .begin(BeginUpload {
                tenant_id: tenant,
                video_id: VideoId::new(),
                version: ObjectVersion::new(1).expect("version"),
                file_name: "source.webm".into(),
                expected_size: size(6),
                content_type: ContentType::parse("video/webm").expect("content type"),
                checksum: None,
            })
            .await
            .expect("begin");
        let part = coordinator
            .upload_part(
                tenant,
                &mut session,
                PartNumber::new(1).expect("part"),
                size(0),
                b"abc".to_vec(),
                Some(checksum('a')),
            )
            .await
            .expect("upload part");
        let replay = coordinator
            .upload_part(
                tenant,
                &mut session,
                PartNumber::new(1).expect("part"),
                size(0),
                b"abc".to_vec(),
                Some(checksum('a')),
            )
            .await
            .expect("replay part");
        assert_eq!(part, replay);
        coordinator
            .upload_part(
                tenant,
                &mut session,
                PartNumber::new(2).expect("part"),
                size(3),
                b"def".to_vec(),
                Some(checksum('b')),
            )
            .await
            .expect("upload part two");
        let first = coordinator
            .finalize(tenant, &mut session)
            .await
            .expect("finalize");
        let second = coordinator
            .finalize(tenant, &mut session)
            .await
            .expect("replay finalize");
        assert_eq!(first, second);
        assert_eq!(session.contract.state, UploadState::Complete);
        assert_eq!(
            store
                .head(&session.object_key)
                .await
                .expect("head")
                .expect("object")
                .size,
            size(6)
        );
    }

    #[tokio::test]
    async fn cross_tenant_access_is_hidden_and_changed_replay_conflicts() {
        let store = MemoryObjectStore::default();
        let coordinator = MultipartUploadCoordinator::new(&store);
        let tenant = TenantId::new();
        let mut session = coordinator
            .begin(BeginUpload {
                tenant_id: tenant,
                video_id: VideoId::new(),
                version: ObjectVersion::new(1).expect("version"),
                file_name: "source.webm".into(),
                expected_size: size(3),
                content_type: ContentType::parse("video/webm").expect("content type"),
                checksum: None,
            })
            .await
            .expect("begin");
        assert_eq!(
            coordinator.finalize(TenantId::new(), &mut session).await,
            Err(ApplicationError::NotFound)
        );
        coordinator
            .upload_part(
                tenant,
                &mut session,
                PartNumber::new(1).expect("part"),
                size(0),
                b"abc".to_vec(),
                Some(checksum('a')),
            )
            .await
            .expect("upload");
        assert_eq!(
            coordinator
                .upload_part(
                    tenant,
                    &mut session,
                    PartNumber::new(1).expect("part"),
                    size(0),
                    b"xyz".to_vec(),
                    Some(checksum('b')),
                )
                .await,
            Err(ApplicationError::Conflict)
        );
    }
}
