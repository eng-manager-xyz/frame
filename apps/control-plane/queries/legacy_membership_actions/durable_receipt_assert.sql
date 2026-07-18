INSERT INTO legacy_membership_action_assertions_v1(
  operation_id, assertion_kind, expected_count, actual_count
)
VALUES (
  ?1, 'durable_receipt', 1,
  (
    SELECT COUNT(*)
    FROM legacy_membership_action_operations_v1 operation
    JOIN legacy_membership_action_receipts_v1 receipt
      ON receipt.operation_id = operation.operation_id
    JOIN legacy_membership_action_effects_v1 effect
      ON effect.operation_id = operation.operation_id
     AND effect.organization_id = operation.organization_id
    WHERE operation.operation_id = ?1
      AND operation.organization_id = ?2
      AND operation.actor_id = ?3
      AND operation.action = ?4
      AND operation.request_digest = ?5
      AND operation.state = 'complete'
      AND effect.authority_subject_count = (
        SELECT COUNT(*) FROM legacy_membership_action_authority_subjects_v1
        WHERE operation_id = operation.operation_id
      )
      AND receipt.matching_before = (
        SELECT COUNT(*) FROM legacy_membership_action_previous_members_v1
        WHERE operation_id = operation.operation_id
      ) + CASE WHEN receipt.result_kind = 'organization_invite_removed' THEN 1 ELSE 0 END
      AND receipt.matching_after = (
        CASE receipt.result_kind
          WHEN 'space_members_added' THEN receipt.matching_before + receipt.inserted_rows
          WHEN 'space_members_removed' THEN 0
          WHEN 'space_member_removed' THEN 0
          ELSE (
            SELECT COUNT(*) FROM legacy_membership_action_final_members_v1
            WHERE operation_id = operation.operation_id
          )
        END
      )
      AND EXISTS (
        SELECT 1 FROM legacy_membership_action_audit_events_v1 audit
        WHERE audit.operation_id = operation.operation_id
          AND audit.organization_id = operation.organization_id
          AND audit.actor_id = operation.actor_id
          AND audit.action = operation.action
          AND audit.outcome = 'allow'
      )
      AND EXISTS (
        SELECT 1 FROM legacy_membership_action_proof_consumptions_v1 proof
        WHERE proof.mutation_grant_id = ?6
          AND proof.session_id = ?7
          AND proof.actor_id = operation.actor_id
          AND proof.related_operation_id = operation.operation_id
          AND proof.organization_id = operation.organization_id
          AND proof.action = operation.action
          AND proof.request_digest = operation.request_digest
          AND proof.outcome = ?8
      )
      AND NOT EXISTS (
        SELECT 1
        FROM legacy_membership_action_authority_subjects_v1 subject
        LEFT JOIN legacy_membership_authority_generations_v1 generation
          ON generation.organization_id = operation.organization_id
         AND generation.user_id = subject.user_id
         AND generation.generation = subject.generation_after
         AND generation.last_operation_id = operation.operation_id
        WHERE subject.operation_id = operation.operation_id
          AND generation.user_id IS NULL
      )
      AND NOT EXISTS (
        SELECT 1
        FROM legacy_membership_action_revoked_grants_v1 revoked
        JOIN auth_session_mutation_grants_v2 live
          ON live.id = revoked.mutation_grant_id
        WHERE revoked.operation_id = operation.operation_id
      )
  )
)
