//! Exact HTTP carriers for Cap's retained download, playlist, object, and upload routes.

use std::collections::BTreeMap;

use frame_application::{
    LEGACY_CORE_STORAGE_CAPABILITY_TTL_SECONDS, LEGACY_CORE_STORAGE_MAX_BODY_BYTES,
    LegacyMultipartAbortInputV1, LegacyMultipartCompleteInputV1, LegacyMultipartInitiateInputV1,
    LegacyMultipartPresignPartInputV1, LegacyPlaylistQueryV1, LegacyPlaylistVideoTypeV1,
    LegacyRecordingCompleteInputV1, LegacySignedUploadBatchInputV1, LegacySignedUploadInputV1,
    LegacyStorageObjectQueryV1, legacy_download_platform,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::Sha256;
use url::Url;
use worker::{Bucket, Env, Include, Range, Request, Response, Result, send::IntoSendFuture};

use crate::{
    direct_upload_signer,
    legacy_core_storage_runtime::{
        D1LegacyCoreStorageV1, LegacyCoreStorageFailureV1, LegacyCoreStorageReadAuthorityV1,
    },
    legacy_extension_auth_web_runtime::{
        LegacyExtensionHttpFailureV1, optional_session_actor, required_actor,
    },
};

const CACHE_CONTROL: &str = "no-store, no-cache, must-revalidate, proxy-revalidate";
const MAX_PLAYLIST_OBJECTS: u32 = 1_000;
const MAX_SEGMENTS: usize = 10_000;

pub(crate) fn download_response(request: &Request) -> Result<Response> {
    let user_agent = request.headers().get("user-agent")?.unwrap_or_default();
    let client_platform = request
        .headers()
        .get("sec-ch-ua-platform")?
        .unwrap_or_default();
    let platform = legacy_download_platform(&user_agent, &client_platform);
    let mut target = request.url()?;
    target.set_path(&format!("/download/{}", platform.path_segment()));
    target.set_query(None);
    target.set_fragment(None);
    Response::redirect_with_status(target, 307)
}

pub(crate) async fn playlist_response(
    request: &Request,
    env: &Env,
    now_ms: i64,
    head_only: bool,
) -> Result<Response> {
    let query = match parse_playlist_query(&request.url()?) {
        Some(query) => query,
        None => return failure_response(LegacyCoreStorageFailureV1::Invalid),
    };
    let actor = match optional_session_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(failure) => return auth_failure(failure),
    };
    let database = env.d1("DB")?;
    let authority = match D1LegacyCoreStorageV1::new(&database)
        .read_authority(
            actor.as_ref().map(|actor| actor.id.as_str()),
            &query.video_id,
            false,
            now_ms,
        )
        .await
    {
        Ok(authority) => authority,
        Err(failure) => return failure_response(failure),
    };
    let Some(signer) = direct_upload_signer(env) else {
        return failure_response(LegacyCoreStorageFailureV1::Unavailable);
    };
    let bucket = env.bucket("RECORDINGS")?;
    match playlist(&bucket, &signer, &authority, &query, now_ms, head_only).await {
        Ok(response) => Ok(response),
        Err(failure) => failure_response(failure),
    }
}

pub(crate) async fn storage_object_response(
    request: &Request,
    env: &Env,
    now_ms: i64,
    head_only: bool,
) -> Result<Response> {
    let query = match parse_storage_object_query(&request.url()?) {
        Some(query) => query,
        None => return failure_response(LegacyCoreStorageFailureV1::Invalid),
    };
    let actor = match optional_session_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(failure) => return auth_failure(failure),
    };
    let exact_token = match query.token.as_deref() {
        Some(token) => verify_storage_token(env, token, &query.video_id, &query.key, now_ms),
        None => false,
    };
    let database = env.d1("DB")?;
    let authority = match D1LegacyCoreStorageV1::new(&database)
        .read_authority(
            actor.as_ref().map(|actor| actor.id.as_str()),
            &query.video_id,
            exact_token,
            now_ms,
        )
        .await
    {
        Ok(authority) if authority.admits_key(&query.key) => authority,
        Ok(_)
        | Err(LegacyCoreStorageFailureV1::Forbidden | LegacyCoreStorageFailureV1::NotFound) => {
            return failure_response(LegacyCoreStorageFailureV1::NotFound);
        }
        Err(failure) => return failure_response(failure),
    };
    let _authority = authority;
    let bucket = env.bucket("RECORDINGS")?;
    match proxy_object(&bucket, request, &query.key, head_only).await {
        Ok(response) => Ok(response),
        Err(failure) => failure_response(failure),
    }
}

pub(crate) async fn multipart_initiate_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actor = match authenticated_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(response) => return Ok(response),
    };
    let input = match decode_json::<LegacyMultipartInitiateInputV1>(request).await? {
        Ok(input) => input,
        Err(failure) => return failure_response(failure),
    };
    let idempotency = request.headers().get("idempotency-key")?;
    let database = env.d1("DB")?;
    let bucket = env.bucket("RECORDINGS")?;
    mutation_result(
        D1LegacyCoreStorageV1::new(&database)
            .initiate(&actor, &input, idempotency.as_deref(), &bucket, now_ms)
            .await,
    )
}

pub(crate) async fn multipart_presign_part_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actor = match authenticated_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(response) => return Ok(response),
    };
    let input = match decode_json::<LegacyMultipartPresignPartInputV1>(request).await? {
        Ok(input) => input,
        Err(failure) => return failure_response(failure),
    };
    let Some(signer) = direct_upload_signer(env) else {
        return failure_response(LegacyCoreStorageFailureV1::Unavailable);
    };
    let idempotency = request.headers().get("idempotency-key")?;
    let database = env.d1("DB")?;
    mutation_result(
        D1LegacyCoreStorageV1::new(&database)
            .presign_part(&actor, &input, idempotency.as_deref(), &signer, now_ms)
            .await,
    )
}

pub(crate) async fn multipart_abort_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actor = match authenticated_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(response) => return Ok(response),
    };
    let input = match decode_json::<LegacyMultipartAbortInputV1>(request).await? {
        Ok(input) => input,
        Err(failure) => return failure_response(failure),
    };
    let idempotency = request.headers().get("idempotency-key")?;
    let database = env.d1("DB")?;
    let bucket = env.bucket("RECORDINGS")?;
    mutation_result(
        D1LegacyCoreStorageV1::new(&database)
            .abort(&actor, &input, idempotency.as_deref(), &bucket, now_ms)
            .await,
    )
}

pub(crate) async fn multipart_complete_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actor = match authenticated_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(response) => return Ok(response),
    };
    let input = match decode_json::<LegacyMultipartCompleteInputV1>(request).await? {
        Ok(input) => input,
        Err(failure) => return failure_response(failure),
    };
    let idempotency = request.headers().get("idempotency-key")?;
    let database = env.d1("DB")?;
    let bucket = env.bucket("RECORDINGS")?;
    mutation_result(
        D1LegacyCoreStorageV1::new(&database)
            .complete(&actor, &input, idempotency.as_deref(), &bucket, now_ms)
            .await,
    )
}

pub(crate) async fn signed_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actor = match authenticated_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(response) => return Ok(response),
    };
    let input = match decode_json::<LegacySignedUploadInputV1>(request).await? {
        Ok(input) => input,
        Err(failure) => return failure_response(failure),
    };
    let Some(signer) = direct_upload_signer(env) else {
        return failure_response(LegacyCoreStorageFailureV1::Unavailable);
    };
    let idempotency = request.headers().get("idempotency-key")?;
    let database = env.d1("DB")?;
    mutation_result(
        D1LegacyCoreStorageV1::new(&database)
            .signed(&actor, &input, idempotency.as_deref(), &signer, now_ms)
            .await,
    )
}

pub(crate) async fn signed_batch_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actor = match authenticated_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(response) => return Ok(response),
    };
    let input = match decode_json::<LegacySignedUploadBatchInputV1>(request).await? {
        Ok(input) => input,
        Err(failure) => return failure_response(failure),
    };
    let Some(signer) = direct_upload_signer(env) else {
        return failure_response(LegacyCoreStorageFailureV1::Unavailable);
    };
    let idempotency = request.headers().get("idempotency-key")?;
    let database = env.d1("DB")?;
    mutation_result(
        D1LegacyCoreStorageV1::new(&database)
            .signed_batch(&actor, &input, idempotency.as_deref(), &signer, now_ms)
            .await,
    )
}

pub(crate) async fn recording_complete_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actor = match authenticated_actor(request, env, now_ms).await? {
        Ok(actor) => actor,
        Err(response) => return Ok(response),
    };
    let input = match decode_json::<LegacyRecordingCompleteInputV1>(request).await? {
        Ok(input) => input,
        Err(failure) => return failure_response(failure),
    };
    let idempotency = request.headers().get("idempotency-key")?;
    let database = env.d1("DB")?;
    mutation_result(
        D1LegacyCoreStorageV1::new(&database)
            .recording_complete(&actor, &input, idempotency.as_deref(), now_ms)
            .await,
    )
}

async fn playlist(
    bucket: &Bucket,
    signer: &crate::r2_direct_upload::R2DirectPutSigner,
    authority: &LegacyCoreStorageReadAuthorityV1,
    query: &LegacyPlaylistQueryV1,
    now_ms: i64,
    head_only: bool,
) -> std::result::Result<Response, LegacyCoreStorageFailureV1> {
    if query.video_type == LegacyPlaylistVideoTypeV1::RawPreview {
        if let Some(key) = authority.raw_file_key.as_deref() {
            return signed_redirect(signer, key, now_ms);
        }
        if authority.source_type != "webMP4" {
            return Err(LegacyCoreStorageFailureV1::NotFound);
        }
        for subpath in ["raw-upload.mp4", "raw-upload.webm"] {
            let key = format!("{}{subpath}", authority.object_prefix);
            if bucket
                .head(&key)
                .into_send()
                .await
                .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
                .is_some_and(|object| object.size() > 0)
            {
                return signed_redirect(signer, &key, now_ms);
            }
        }
        return Err(LegacyCoreStorageFailureV1::NotFound);
    }
    if matches!(
        query.video_type,
        LegacyPlaylistVideoTypeV1::SegmentsMaster
            | LegacyPlaylistVideoTypeV1::SegmentsVideo
            | LegacyPlaylistVideoTypeV1::SegmentsAudio
    ) {
        return segments_playlist(bucket, signer, authority, query, now_ms, head_only).await;
    }
    if query.file_type.as_deref() == Some("transcription") {
        return object_text(
            bucket,
            &format!("{}transcription.vtt", authority.object_prefix),
            "text/vtt",
            head_only,
        )
        .await;
    }
    if query.file_type.as_deref() == Some("enhanced-audio") {
        return signed_redirect(
            signer,
            &format!("{}enhanced-audio.mp3", authority.object_prefix),
            now_ms,
        );
    }
    if matches!(authority.source_type.as_str(), "desktopMP4" | "webMP4")
        || query.video_type == LegacyPlaylistVideoTypeV1::Mp4
    {
        return signed_redirect(
            signer,
            &format!("{}result.mp4", authority.object_prefix),
            now_ms,
        );
    }
    if authority.source_type == "MediaConvert" {
        return signed_redirect(
            signer,
            &format!("{}output/video_recording_000.m3u8", authority.object_prefix),
            now_ms,
        );
    }
    if authority.source_type == "local" {
        let key = format!("{}combined-source/stream.m3u8", authority.object_prefix);
        let object = bucket
            .get(&key)
            .execute()
            .into_send()
            .await
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
            .ok_or(LegacyCoreStorageFailureV1::NotFound)?;
        let text = object
            .body()
            .ok_or(LegacyCoreStorageFailureV1::Corrupt)?
            .text()
            .into_send()
            .await
            .map_err(|_| LegacyCoreStorageFailureV1::Corrupt)?;
        if text.len() > 4 * 1_024 * 1_024 {
            return Err(LegacyCoreStorageFailureV1::Corrupt);
        }
        let mut lines = Vec::new();
        for line in text.split('\n') {
            if line.ends_with(".ts") {
                lines.push(signed_url(
                    signer,
                    &format!("{}combined-source/{line}", authority.object_prefix),
                    now_ms,
                )?);
            } else {
                lines.push(line.to_owned());
            }
        }
        let rendered = lines.join("\n");
        return playlist_text(&rendered, head_only);
    }
    generated_playlist(bucket, signer, authority, query, now_ms, head_only).await
}

async fn segments_playlist(
    bucket: &Bucket,
    signer: &crate::r2_direct_upload::R2DirectPutSigner,
    authority: &LegacyCoreStorageReadAuthorityV1,
    query: &LegacyPlaylistQueryV1,
    now_ms: i64,
    head_only: bool,
) -> std::result::Result<Response, LegacyCoreStorageFailureV1> {
    let key = format!("{}segments/manifest.json", authority.object_prefix);
    let object = bucket
        .get(&key)
        .execute()
        .into_send()
        .await
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
        .ok_or(LegacyCoreStorageFailureV1::NotFound)?;
    let bytes = object
        .body()
        .ok_or(LegacyCoreStorageFailureV1::Corrupt)?
        .bytes()
        .into_send()
        .await
        .map_err(|_| LegacyCoreStorageFailureV1::Corrupt)?;
    if bytes.len() > 2 * 1_024 * 1_024 {
        return Err(LegacyCoreStorageFailureV1::Corrupt);
    }
    let manifest: SegmentManifestV1 =
        serde_json::from_slice(&bytes).map_err(|_| LegacyCoreStorageFailureV1::Corrupt)?;
    if !manifest.valid() || (query.require_complete && !manifest.is_complete) {
        return Err(LegacyCoreStorageFailureV1::NotFound);
    }
    if query.video_type == LegacyPlaylistVideoTypeV1::SegmentsMaster {
        if !manifest.video_init_uploaded || manifest.video_segments.is_empty() {
            return Err(LegacyCoreStorageFailureV1::NotFound);
        }
        let suffix = if query.require_complete {
            "&requireComplete=1"
        } else {
            ""
        };
        let mut text = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n".to_owned();
        if manifest.audio_init_uploaded && !manifest.audio_segments.is_empty() {
            text.push_str(&format!("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"default\",DEFAULT=YES,AUTOSELECT=YES,URI=\"/api/playlist?videoId={}&videoType=segments-audio{suffix}\"\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,AUDIO=\"audio\"\n", authority.legacy_video_id));
        } else {
            text.push_str("#EXT-X-STREAM-INF:BANDWIDTH=2000000\n");
        }
        text.push_str(&format!(
            "/api/playlist?videoId={}&videoType=segments-video{suffix}\n",
            authority.legacy_video_id
        ));
        return playlist_text(&text, head_only);
    }
    let video = query.video_type == LegacyPlaylistVideoTypeV1::SegmentsVideo;
    let (init_uploaded, entries, media) = if video {
        (
            manifest.video_init_uploaded,
            manifest.video_segments,
            "video",
        )
    } else {
        (
            manifest.audio_init_uploaded,
            manifest.audio_segments,
            "audio",
        )
    };
    if !init_uploaded || entries.is_empty() {
        return Err(LegacyCoreStorageFailureV1::NotFound);
    }
    let normalized = entries
        .into_iter()
        .map(SegmentEntryV1::normalized)
        .collect::<Option<Vec<_>>>()
        .ok_or(LegacyCoreStorageFailureV1::Corrupt)?;
    let target_duration = normalized
        .iter()
        .map(|entry| entry.duration.ceil() as u64)
        .max()
        .unwrap_or(1)
        .max(1);
    let init_url = signed_url(
        signer,
        &format!("{}segments/{media}/init.mp4", authority.object_prefix),
        now_ms,
    )?;
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target_duration}\n#EXT-X-MEDIA-SEQUENCE:0\n"
    );
    if manifest.is_complete {
        text.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");
    }
    text.push_str(&format!("#EXT-X-MAP:URI=\"{init_url}\"\n"));
    for entry in normalized {
        let url = signed_url(
            signer,
            &format!(
                "{}segments/{media}/segment_{:03}.m4s",
                authority.object_prefix, entry.index
            ),
            now_ms,
        )?;
        text.push_str(&format!("#EXTINF:{:.3},\n{url}\n", entry.duration));
    }
    if manifest.is_complete {
        text.push_str("#EXT-X-ENDLIST\n");
    }
    playlist_text(&text, head_only)
}

async fn generated_playlist(
    bucket: &Bucket,
    signer: &crate::r2_direct_upload::R2DirectPutSigner,
    authority: &LegacyCoreStorageReadAuthorityV1,
    query: &LegacyPlaylistQueryV1,
    now_ms: i64,
    head_only: bool,
) -> std::result::Result<Response, LegacyCoreStorageFailureV1> {
    if query.video_type == LegacyPlaylistVideoTypeV1::Master {
        let video_prefix = format!("{}video/", authority.object_prefix);
        let audio_prefix = format!("{}audio/", authority.object_prefix);
        let videos = list_objects(bucket, &video_prefix, 1).await?;
        let first = videos.first().ok_or(LegacyCoreStorageFailureV1::NotFound)?;
        let video_metadata = bucket
            .head(first)
            .into_send()
            .await
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
            .ok_or(LegacyCoreStorageFailureV1::NotFound)?
            .custom_metadata()
            .map_err(|_| LegacyCoreStorageFailureV1::Corrupt)?;
        let has_audio = !list_objects(bucket, &audio_prefix, 1).await?.is_empty();
        let resolution = video_metadata.get("resolution").map_or("", String::as_str);
        let bandwidth = video_metadata.get("bandwidth").map_or("", String::as_str);
        let text =
            render_master_playlist(&authority.legacy_video_id, resolution, bandwidth, has_audio);
        return playlist_text(&text, head_only);
    }
    let media = match query.video_type {
        LegacyPlaylistVideoTypeV1::Video => "video",
        LegacyPlaylistVideoTypeV1::Audio => "audio",
        _ => return Err(LegacyCoreStorageFailureV1::NotFound),
    };
    let limit = if query.thumbnail {
        1
    } else {
        MAX_PLAYLIST_OBJECTS
    };
    let keys = list_objects(
        bucket,
        &format!("{}{media}/", authority.object_prefix),
        limit,
    )
    .await?;
    let mut segments = Vec::with_capacity(keys.len());
    for key in keys {
        let object = bucket
            .head(&key)
            .into_send()
            .await
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
            .ok_or(LegacyCoreStorageFailureV1::NotFound)?;
        let metadata = object
            .custom_metadata()
            .map_err(|_| LegacyCoreStorageFailureV1::Corrupt)?;
        let duration = metadata.get("duration").map_or("", String::as_str);
        let url = signed_url(signer, &key, now_ms)?;
        segments.push((url, duration.to_owned()));
    }
    let text = render_media_playlist(&segments);
    playlist_text(&text, head_only)
}

fn render_master_playlist(
    video_id: &str,
    resolution: &str,
    bandwidth: &str,
    has_audio: bool,
) -> String {
    let mut text = "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-INDEPENDENT-SEGMENTS\n".to_owned();
    if has_audio {
        text.push_str(&format!("#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"Audio\",DEFAULT=YES,AUTOSELECT=YES,LANGUAGE=\"en\",URI=\"/api/playlist?videoId={video_id}&videoType=audio\"\n"));
    }
    text.push_str(&format!(
        "#EXT-X-STREAM-INF:BANDWIDTH={bandwidth},RESOLUTION={resolution}"
    ));
    if has_audio {
        text.push_str(",AUDIO=\"audio\"");
    }
    text.push_str(&format!(
        "\n/api/playlist?videoId={video_id}&videoType=video\n"
    ));
    text
}

fn render_media_playlist(segments: &[(String, String)]) -> String {
    let mut text = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:5\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n".to_owned();
    for (url, duration) in segments {
        text.push_str(&format!("#EXTINF:{duration},\n{url}\n"));
    }
    text.push_str("#EXT-X-ENDLIST");
    text
}

async fn list_objects(
    bucket: &Bucket,
    prefix: &str,
    limit: u32,
) -> std::result::Result<Vec<String>, LegacyCoreStorageFailureV1> {
    let listed = bucket
        .list()
        .prefix(prefix)
        .limit(limit)
        .include(vec![Include::CustomMetadata, Include::HttpMetadata])
        .execute()
        .into_send()
        .await
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
    if listed.truncated() && limit == MAX_PLAYLIST_OBJECTS {
        return Err(LegacyCoreStorageFailureV1::Unavailable);
    }
    let keys = listed
        .objects()
        .into_iter()
        .map(|object| object.key())
        .collect::<Vec<_>>();
    if keys.iter().any(|key| !key.starts_with(prefix)) {
        return Err(LegacyCoreStorageFailureV1::Corrupt);
    }
    Ok(keys)
}

async fn object_text(
    bucket: &Bucket,
    key: &str,
    content_type: &str,
    head_only: bool,
) -> std::result::Result<Response, LegacyCoreStorageFailureV1> {
    let object = bucket
        .get(key)
        .execute()
        .into_send()
        .await
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
        .ok_or(LegacyCoreStorageFailureV1::NotFound)?;
    let text = object
        .body()
        .ok_or(LegacyCoreStorageFailureV1::Corrupt)?
        .text()
        .into_send()
        .await
        .map_err(|_| LegacyCoreStorageFailureV1::Corrupt)?;
    text_response(&text, content_type, head_only)
}

async fn proxy_object(
    bucket: &Bucket,
    request: &Request,
    key: &str,
    head_only: bool,
) -> std::result::Result<Response, LegacyCoreStorageFailureV1> {
    let head = bucket
        .head(key)
        .into_send()
        .await
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
        .ok_or(LegacyCoreStorageFailureV1::NotFound)?;
    let total = head.size();
    let requested = request
        .headers()
        .get("range")
        .map_err(|_| LegacyCoreStorageFailureV1::Invalid)?;
    let range = requested
        .as_deref()
        .map(|value| parse_range(value, total))
        .transpose()?;
    let (start, length) = range.as_ref().map_or((0, total), |range| match range {
        Range::OffsetWithLength { offset, length } => (*offset, *length),
        _ => unreachable!("range parser only emits offset+length"),
    });
    let mut response = if head_only {
        Response::empty().map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
    } else {
        let object = match range.clone() {
            Some(range) => bucket.get(key).range(range).execute(),
            None => bucket.get(key).execute(),
        }
        .into_send()
        .await
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
        .ok_or(LegacyCoreStorageFailureV1::NotFound)?;
        let body = object
            .body()
            .ok_or(LegacyCoreStorageFailureV1::Corrupt)?
            .response_body()
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
        Response::from_body(body).map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
    };
    if range.is_some() {
        response = response.with_status(206);
        response
            .headers_mut()
            .set(
                "content-range",
                &format!("bytes {start}-{}/{total}", start + length - 1),
            )
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
    }
    let metadata = head.http_metadata();
    let headers = response.headers_mut();
    headers
        .set(
            "content-type",
            metadata
                .content_type
                .as_deref()
                .unwrap_or("application/octet-stream"),
        )
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
    headers
        .set("content-length", &length.to_string())
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
    headers
        .set("accept-ranges", "bytes")
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
    cache_headers(&mut response).map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
    Ok(response)
}

fn parse_range(value: &str, total: u64) -> std::result::Result<Range, LegacyCoreStorageFailureV1> {
    let raw = value
        .strip_prefix("bytes=")
        .ok_or(LegacyCoreStorageFailureV1::RangeNotSatisfiable)?;
    if raw.contains(',') || total == 0 {
        return Err(LegacyCoreStorageFailureV1::RangeNotSatisfiable);
    }
    let (left, right) = raw
        .split_once('-')
        .ok_or(LegacyCoreStorageFailureV1::RangeNotSatisfiable)?;
    let (start, end) = if left.is_empty() {
        let suffix = right
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or(LegacyCoreStorageFailureV1::RangeNotSatisfiable)?;
        (total.saturating_sub(suffix.min(total)), total - 1)
    } else {
        let start = left
            .parse::<u64>()
            .map_err(|_| LegacyCoreStorageFailureV1::RangeNotSatisfiable)?;
        let end = if right.is_empty() {
            total - 1
        } else {
            right
                .parse::<u64>()
                .map_err(|_| LegacyCoreStorageFailureV1::RangeNotSatisfiable)?
                .min(total - 1)
        };
        if start >= total || end < start {
            return Err(LegacyCoreStorageFailureV1::RangeNotSatisfiable);
        }
        (start, end)
    };
    Ok(Range::OffsetWithLength {
        offset: start,
        length: end - start + 1,
    })
}

fn signed_redirect(
    signer: &crate::r2_direct_upload::R2DirectPutSigner,
    key: &str,
    now_ms: i64,
) -> std::result::Result<Response, LegacyCoreStorageFailureV1> {
    let url = signed_url(signer, key, now_ms)?;
    let url = Url::parse(&url).map_err(|_| LegacyCoreStorageFailureV1::Corrupt)?;
    Response::redirect_with_status(url, 302).map_err(|_| LegacyCoreStorageFailureV1::Unavailable)
}

fn signed_url(
    signer: &crate::r2_direct_upload::R2DirectPutSigner,
    key: &str,
    now_ms: i64,
) -> std::result::Result<String, LegacyCoreStorageFailureV1> {
    signer
        .sign_legacy_storage_get(
            key,
            u64::try_from(now_ms).map_err(|_| LegacyCoreStorageFailureV1::Invalid)?,
            LEGACY_CORE_STORAGE_CAPABILITY_TTL_SECONDS,
        )
        .map(|capability| capability.url)
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)
}

fn playlist_text(
    text: &str,
    head_only: bool,
) -> std::result::Result<Response, LegacyCoreStorageFailureV1> {
    text_response(text, "application/vnd.apple.mpegurl", head_only)
}

fn text_response(
    text: &str,
    content_type: &str,
    head_only: bool,
) -> std::result::Result<Response, LegacyCoreStorageFailureV1> {
    let mut response = if head_only {
        Response::empty().map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
    } else {
        Response::from_bytes(text.as_bytes().to_vec())
            .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?
    };
    response
        .headers_mut()
        .set("content-type", content_type)
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
    response
        .headers_mut()
        .set("content-length", &text.len().to_string())
        .map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
    cache_headers(&mut response).map_err(|_| LegacyCoreStorageFailureV1::Unavailable)?;
    Ok(response)
}

fn parse_playlist_query(url: &Url) -> Option<LegacyPlaylistQueryV1> {
    let pairs = exact_query_pairs(url)?;
    LegacyPlaylistQueryV1::parse(
        pairs.get("videoId")?,
        pairs.get("videoType")?,
        pairs.get("requireComplete").map(String::as_str),
        pairs.get("thumbnail").map(String::as_str),
        pairs.get("fileType").map(String::as_str),
    )
}

fn parse_storage_object_query(url: &Url) -> Option<LegacyStorageObjectQueryV1> {
    let pairs = exact_query_pairs(url)?;
    LegacyStorageObjectQueryV1::parse(
        pairs.get("videoId")?,
        pairs.get("key")?,
        pairs.get("token").map(String::as_str),
    )
}

fn exact_query_pairs(url: &Url) -> Option<BTreeMap<String, String>> {
    let mut pairs = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if pairs.insert(key.into_owned(), value.into_owned()).is_some() {
            return None;
        }
    }
    Some(pairs)
}

async fn authenticated_actor(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<std::result::Result<String, Response>> {
    Ok(match required_actor(request, env, now_ms).await? {
        Ok(actor) => Ok(actor.id),
        Err(failure) => Err(auth_failure(failure)?),
    })
}

async fn decode_json<T: DeserializeOwned>(
    request: &mut Request,
) -> Result<std::result::Result<T, LegacyCoreStorageFailureV1>> {
    let Some(content_type) = request.headers().get("content-type")? else {
        return Ok(Err(LegacyCoreStorageFailureV1::Invalid));
    };
    if content_type.split(';').next().map(str::trim) != Some("application/json")
        || request
            .headers()
            .get("content-encoding")?
            .is_some_and(|value| value != "identity")
    {
        return Ok(Err(LegacyCoreStorageFailureV1::Invalid));
    }
    let declared = request
        .headers()
        .get("content-length")?
        .and_then(|value| value.parse::<usize>().ok());
    if declared.is_some_and(|value| value == 0 || value > LEGACY_CORE_STORAGE_MAX_BODY_BYTES) {
        return Ok(Err(LegacyCoreStorageFailureV1::Invalid));
    }
    let bytes =
        match crate::read_bounded_legacy_body(request, LEGACY_CORE_STORAGE_MAX_BODY_BYTES).await {
            Ok(bytes) => bytes,
            Err(()) => return Ok(Err(LegacyCoreStorageFailureV1::Invalid)),
        };
    if bytes.is_empty() || declared.is_some_and(|value| value != bytes.len()) {
        return Ok(Err(LegacyCoreStorageFailureV1::Invalid));
    }
    Ok(serde_json::from_slice(&bytes).map_err(|_| LegacyCoreStorageFailureV1::Invalid))
}

fn mutation_result(
    result: std::result::Result<Value, LegacyCoreStorageFailureV1>,
) -> Result<Response> {
    match result {
        Ok(value) => exact_json(200, value),
        Err(failure) => failure_response(failure),
    }
}

fn auth_failure(failure: LegacyExtensionHttpFailureV1) -> Result<Response> {
    match failure {
        LegacyExtensionHttpFailureV1::BadRequest | LegacyExtensionHttpFailureV1::Unauthorized => {
            failure_response(LegacyCoreStorageFailureV1::Unauthorized)
        }
        LegacyExtensionHttpFailureV1::Internal => {
            failure_response(LegacyCoreStorageFailureV1::Unavailable)
        }
    }
}

fn failure_response(failure: LegacyCoreStorageFailureV1) -> Result<Response> {
    let (status, error) = failure_projection(failure);
    let mut response = if failure == LegacyCoreStorageFailureV1::RangeNotSatisfiable {
        Response::from_bytes(b"Range not satisfiable".to_vec())?.with_status(status)
    } else {
        Response::from_json(&json!({"error": error}))?.with_status(status)
    };
    cache_headers(&mut response)?;
    Ok(response)
}

const fn failure_projection(failure: LegacyCoreStorageFailureV1) -> (u16, &'static str) {
    match failure {
        LegacyCoreStorageFailureV1::Invalid => (400, "Bad request"),
        LegacyCoreStorageFailureV1::Unauthorized => (401, "Unauthorized"),
        LegacyCoreStorageFailureV1::Forbidden => (403, "Forbidden"),
        LegacyCoreStorageFailureV1::NotFound => (404, "Not found"),
        LegacyCoreStorageFailureV1::Conflict => (409, "Conflict"),
        LegacyCoreStorageFailureV1::RangeNotSatisfiable => (416, "Range not satisfiable"),
        LegacyCoreStorageFailureV1::ProviderGated => (503, "provider_execution"),
        LegacyCoreStorageFailureV1::Corrupt | LegacyCoreStorageFailureV1::Unavailable => {
            (500, "Internal server error")
        }
    }
}

fn exact_json(status: u16, value: Value) -> Result<Response> {
    let mut response = Response::from_json(&value)?.with_status(status);
    cache_headers(&mut response)?;
    Ok(response)
}

fn cache_headers(response: &mut Response) -> Result<()> {
    let headers = response.headers_mut();
    headers.set("cache-control", CACHE_CONTROL)?;
    headers.set("pragma", "no-cache")?;
    headers.set("expires", "0")?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StorageTokenPayloadV1 {
    video_id: String,
    key: String,
    expires_at: i64,
}

fn verify_storage_token(env: &Env, token: &str, video_id: &str, key: &str, now_ms: i64) -> bool {
    let Some((payload, signature)) = token.split_once('.') else {
        return false;
    };
    if signature.contains('.') || payload.len() > 8_192 || signature.len() > 128 {
        return false;
    }
    let Some(secret) = env_secret(env, "FRAME_LEGACY_CORE_STORAGE_NEXTAUTH_SECRET") else {
        return false;
    };
    let Some(signature) = decode_base64url(signature) else {
        return false;
    };
    let mut verification =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    verification.update(payload.as_bytes());
    if verification.verify_slice(&signature).is_err() {
        return false;
    }
    let Some(payload) = decode_base64url(payload) else {
        return false;
    };
    let Ok(payload) = serde_json::from_slice::<StorageTokenPayloadV1>(&payload) else {
        return false;
    };
    payload.video_id == video_id
        && payload.key == key
        && payload.expires_at >= now_ms
        && payload.expires_at <= now_ms.saturating_add(24 * 60 * 60 * 1_000)
}

fn decode_base64url(value: &str) -> Option<Vec<u8>> {
    if value.is_empty()
        || value.len() > 16_384
        || value.len() % 4 == 1
        || value.bytes().any(|byte| !is_base64url(byte))
    {
        return None;
    }
    let mut output = Vec::with_capacity(value.len() * 3 / 4 + 2);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    for byte in value.bytes() {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        accumulator = (accumulator << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1_u32 << bits) - 1;
        }
    }
    if bits > 0 && accumulator != 0 {
        return None;
    }
    Some(output)
}

const fn is_base64url(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn env_secret(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|value| value.to_string())
        .or_else(|_| env.var(name).map(|value| value.to_string()))
        .ok()
        .filter(|value| value.len() >= 32 && value.len() <= 4_096)
}

#[derive(Debug, Deserialize)]
struct SegmentManifestV1 {
    version: f64,
    video_init_uploaded: bool,
    audio_init_uploaded: bool,
    video_segments: Vec<SegmentEntryV1>,
    audio_segments: Vec<SegmentEntryV1>,
    is_complete: bool,
}

impl SegmentManifestV1 {
    fn valid(&self) -> bool {
        self.version.is_finite()
            && self.version >= 0.0
            && self.video_segments.len() <= MAX_SEGMENTS
            && self.audio_segments.len() <= MAX_SEGMENTS
            && self.video_segments.iter().all(SegmentEntryV1::valid)
            && self.audio_segments.iter().all(SegmentEntryV1::valid)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum SegmentEntryV1 {
    Index(u64),
    Detailed { index: u64, duration: f64 },
}

impl SegmentEntryV1 {
    fn valid(&self) -> bool {
        match self {
            Self::Index(index) => *index <= 999_999,
            Self::Detailed { index, duration } => {
                *index <= 999_999 && duration.is_finite() && *duration > 0.0 && *duration <= 3_600.0
            }
        }
    }

    fn normalized(self) -> Option<NormalizedSegmentV1> {
        let (index, duration) = match self {
            Self::Index(index) => (index, 3.0),
            Self::Detailed { index, duration } => (index, duration),
        };
        (index <= 999_999 && duration.is_finite() && duration > 0.0 && duration <= 3_600.0)
            .then_some(NormalizedSegmentV1 { index, duration })
    }
}

struct NormalizedSegmentV1 {
    index: u64,
    duration: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_parser_has_get_head_parity_and_rejects_multirange() {
        assert_eq!(
            parse_range("bytes=2-5", 10),
            Ok(Range::OffsetWithLength {
                offset: 2,
                length: 4
            })
        );
        assert_eq!(
            parse_range("bytes=-3", 10),
            Ok(Range::OffsetWithLength {
                offset: 7,
                length: 3
            })
        );
        assert_eq!(
            parse_range("bytes=4-", 10),
            Ok(Range::OffsetWithLength {
                offset: 4,
                length: 6
            })
        );
        assert_eq!(
            parse_range("bytes=0-1,4-5", 10),
            Err(LegacyCoreStorageFailureV1::RangeNotSatisfiable)
        );
    }

    #[test]
    fn cap_base64url_decoder_is_canonical() {
        assert_eq!(
            decode_base64url("eyJrZXkiOiJ2YWx1ZSJ9"),
            Some(br#"{"key":"value"}"#.to_vec())
        );
        assert_eq!(decode_base64url("bad="), None);
        assert_eq!(decode_base64url("A"), None);
    }

    #[test]
    fn failure_projection_preserves_provider_gate_and_non_disclosure() {
        assert_eq!(
            failure_projection(LegacyCoreStorageFailureV1::ProviderGated),
            (503, "provider_execution")
        );
        assert_eq!(
            failure_projection(LegacyCoreStorageFailureV1::NotFound),
            (404, "Not found")
        );
    }

    #[test]
    fn generated_hls_text_matches_the_pinned_cap_helpers() {
        assert_eq!(
            render_master_playlist("video", "1920x1080", "2000000", true),
            "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"Audio\",DEFAULT=YES,AUTOSELECT=YES,LANGUAGE=\"en\",URI=\"/api/playlist?videoId=video&videoType=audio\"\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1920x1080,AUDIO=\"audio\"\n/api/playlist?videoId=video&videoType=video\n"
        );
        assert_eq!(
            render_media_playlist(&[("https://r2.example/segment.ts".into(), "3.25".into())]),
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:5\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:3.25,\nhttps://r2.example/segment.ts\n#EXT-X-ENDLIST"
        );
        assert_eq!(
            render_media_playlist(&[]),
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:5\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-ENDLIST"
        );
    }
}
