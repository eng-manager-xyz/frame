SELECT data_class, subject_id, export_json FROM (
  SELECT 'video_metadata' AS data_class, id AS subject_id,
    json_object('id',id,'owner_id',owner_id,'privacy',privacy,'metadata',json(metadata_json),
      'comments_enabled',comments_enabled,'created_at_ms',created_at_ms,
      'updated_at_ms',updated_at_ms,'deleted_at_ms',deleted_at_ms,'revision',revision) AS export_json
  FROM videos WHERE organization_id=?1
  UNION ALL
  SELECT 'video_edit', edit.id,
    json_object('id',edit.id,'video_id',edit.video_id,'document_version',edit.document_version,
      'edit_spec',json(edit.edit_spec_json),'created_by_user_id',edit.created_by_user_id,
      'created_at_ms',edit.created_at_ms,'updated_at_ms',edit.updated_at_ms,'revision',edit.revision)
  FROM video_edits edit JOIN videos video ON video.id=edit.video_id WHERE video.organization_id=?1
  UNION ALL
  SELECT 'share', id,
    json_object('id',id,'video_id',video_id,'folder_id',folder_id,'shared_by_user_id',shared_by_user_id,
      'sharing_mode',sharing_mode,'shared_at_ms',shared_at_ms,'revoked_at_ms',revoked_at_ms,'revision',revision)
  FROM shared_videos WHERE organization_id=?1
  UNION ALL
  SELECT 'comment', id,
    json_object('id',id,'video_id',video_id,'parent_comment_id',parent_comment_id,
      'author_kind',CASE WHEN author_user_id IS NULL THEN 'anonymous' ELSE 'user' END,
      'author_user_id',author_user_id,'body',body,'comment_kind',comment_kind,
      'timeline_micros',timeline_micros,'created_at_ms',created_at_ms,
      'updated_at_ms',updated_at_ms,'deleted_at_ms',deleted_at_ms,'revision',revision)
  FROM comments WHERE organization_id=?1
  UNION ALL
  SELECT 'notification', id,
    json_object('id',id,'recipient_user_id',recipient_user_id,'type',type,'data',json(data_json),
      'created_at_ms',created_at_ms,'read_at_ms',read_at_ms)
  FROM notifications WHERE organization_id=?1
  UNION ALL
  SELECT 'outbox', id,
    json_object('id',id,'aggregate_type',aggregate_type,'aggregate_id',aggregate_id,
      'event_type',event_type,'payload',json(payload_json),'state',state,'attempt',attempt,
      'available_at_ms',available_at_ms,'created_at_ms',created_at_ms,
      'delivered_at_ms',delivered_at_ms,'event_sequence',event_sequence,'revision',revision)
  FROM outbox_events WHERE organization_id=?1
  UNION ALL
  SELECT 'storage_object', id,
    json_object('id',id,'integration_id',integration_id,'video_id',video_id,'object_key',object_key,
      'role',role,'object_version',object_version,'state',state,'bytes',bytes,
      'content_type',content_type,'checksum_sha256',checksum_sha256,
      'created_at_ms',created_at_ms,'deleted_at_ms',deleted_at_ms,
      'updated_at_ms',updated_at_ms,'revision',revision)
  FROM storage_objects WHERE organization_id=?1
  UNION ALL
  SELECT 'derivative_job', job_id,
    json_object('job_id',job_id,'executor',executor,'source_object_id',source_object_id,
      'source_version',source_version,'transform_profile',transform_profile,
      'profile_version',profile_version,'output_role',output_role,'output_object_id',output_object_id,
      'output_object_key',output_object_key,'output_checksum',output_checksum,
      'output_content_type',output_content_type,'state',state,'usage_units',usage_units,
      'cost_microcredits',cost_microcredits,'failure_class',failure_class,'revision',revision)
  FROM business_derivative_manifests_v1 WHERE organization_id=?1
  UNION ALL
  SELECT 'upload', id,
    json_object('id',id,'video_id',video_id,'state',state,'expected_bytes',expected_bytes,
      'received_bytes',received_bytes,'source_object_key',source_object_key,
      'source_version',source_version,'content_type',content_type,'checksum_sha256',checksum_sha256,
      'created_at_ms',created_at_ms,'updated_at_ms',updated_at_ms,
      'event_sequence',event_sequence,'revision',revision)
  FROM video_uploads WHERE organization_id=?1
  UNION ALL
  SELECT 'import', id,
    json_object('id',id,'video_id',video_id,'provider',provider,'state',state,
      'error_class',error_class,'created_at_ms',created_at_ms,'updated_at_ms',updated_at_ms,
      'event_sequence',event_sequence,'revision',revision)
  FROM imported_videos WHERE organization_id=?1
  UNION ALL
  SELECT 'developer_app', id,
    json_object('id',id,'owner_user_id',owner_user_id,'name',name,'environment',environment,
      'status',status,'created_at_ms',created_at_ms,'updated_at_ms',updated_at_ms,
      'deleted_at_ms',deleted_at_ms,'revision',revision)
  FROM developer_apps WHERE organization_id=?1
  UNION ALL
  SELECT 'developer_domain', domain.app_id || ':' || domain.domain_ascii,
    json_object('app_id',domain.app_id,'domain_ascii',domain.domain_ascii,
      'created_at_ms',domain.created_at_ms,'verified_at_ms',domain.verified_at_ms,'revision',domain.revision)
  FROM developer_app_domains domain JOIN developer_apps app ON app.id=domain.app_id
  WHERE app.organization_id=?1
  UNION ALL
  SELECT 'developer_video', item.id,
    json_object('id',item.id,'app_id',item.app_id,'video_id',item.video_id,
      'metadata',CASE WHEN item.metadata_json IS NULL THEN NULL ELSE json(item.metadata_json) END,
      'created_at_ms',item.created_at_ms,'updated_at_ms',item.updated_at_ms,
      'deleted_at_ms',item.deleted_at_ms,'revision',item.revision)
  FROM developer_videos item JOIN developer_apps app ON app.id=item.app_id
  WHERE app.organization_id=?1
  UNION ALL
  SELECT 'credit_account', account.id,
    json_object('id',account.id,'app_id',account.app_id,'balance_microcredits',account.balance_microcredits,
      'auto_top_up_enabled',account.auto_top_up_enabled,
      'auto_top_up_threshold_microcredits',account.auto_top_up_threshold_microcredits,
      'created_at_ms',account.created_at_ms,'updated_at_ms',account.updated_at_ms,
      'revision',account.revision,'ledger_sequence',account.ledger_sequence)
  FROM developer_credit_accounts account JOIN developer_apps app ON app.id=account.app_id
  WHERE app.organization_id=?1
  UNION ALL
  SELECT 'credit_transaction', transaction_row.id,
    json_object('id',transaction_row.id,'account_id',transaction_row.account_id,
      'transaction_type',transaction_row.transaction_type,
      'amount_microcredits',transaction_row.amount_microcredits,
      'balance_after_microcredits',transaction_row.balance_after_microcredits,
      'reference_type',transaction_row.reference_type,
      'created_at_ms',transaction_row.created_at_ms,'ledger_sequence',transaction_row.ledger_sequence)
  FROM developer_credit_transactions transaction_row
  JOIN developer_credit_accounts account ON account.id=transaction_row.account_id
  JOIN developer_apps app ON app.id=account.app_id WHERE app.organization_id=?1
  UNION ALL
  SELECT 'usage_ledger', usage.id,
    json_object('id',usage.id,'app_id',usage.app_id,'video_id',usage.video_id,
      'media_job_id',usage.media_job_id,'usage_type',usage.usage_type,'quantity',usage.quantity,
      'microcredits_charged',usage.microcredits_charged,
      'occurred_at_ms',usage.occurred_at_ms,'recorded_at_ms',usage.recorded_at_ms)
  FROM usage_ledger usage LEFT JOIN developer_apps app ON app.id=usage.app_id
  WHERE usage.organization_id=?1 OR (usage.organization_id IS NULL AND app.organization_id=?1)
  UNION ALL
  SELECT 'daily_storage_snapshot', snapshot.app_id || ':' || snapshot.snapshot_day,
    json_object('app_id',snapshot.app_id,'snapshot_day',snapshot.snapshot_day,
      'total_bytes',snapshot.total_bytes,'microcredits_charged',snapshot.microcredits_charged,
      'source_checksum',snapshot.source_checksum,'processed_at_ms',snapshot.processed_at_ms,
      'created_at_ms',snapshot.created_at_ms,'revision',snapshot.revision)
  FROM developer_daily_storage_snapshots snapshot
  JOIN developer_apps app ON app.id=snapshot.app_id WHERE app.organization_id=?1
)
WHERE data_class > ?2 OR (data_class = ?2 AND subject_id > ?3)
ORDER BY data_class, subject_id
LIMIT ?4
