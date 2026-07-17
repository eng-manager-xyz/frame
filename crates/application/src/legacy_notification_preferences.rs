//! Exact application semantics for Cap's notification-preferences read.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyNotificationPreferencesCandidateV1 {
    pause_comments: bool,
    pause_replies: bool,
    pause_views: bool,
    pause_reactions: bool,
    pause_anon_views: bool,
}

impl LegacyNotificationPreferencesCandidateV1 {
    #[must_use]
    pub const fn new(
        pause_comments: bool,
        pause_replies: bool,
        pause_views: bool,
        pause_reactions: bool,
        pause_anon_views: bool,
    ) -> Self {
        Self {
            pause_comments,
            pause_replies,
            pause_views,
            pause_reactions,
            pause_anon_views,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyNotificationPreferencesV1 {
    pause_comments: bool,
    pause_replies: bool,
    pause_views: bool,
    pause_reactions: bool,
    pause_anon_views: bool,
}

impl LegacyNotificationPreferencesV1 {
    /// A missing, null, or schema-invalid Cap preferences value becomes the
    /// complete all-false object. Callers must validate the whole source
    /// notifications object before constructing the candidate.
    #[must_use]
    pub const fn from_validated_source(
        candidate: Option<LegacyNotificationPreferencesCandidateV1>,
    ) -> Self {
        let Some(candidate) = candidate else {
            return Self {
                pause_comments: false,
                pause_replies: false,
                pause_views: false,
                pause_reactions: false,
                pause_anon_views: false,
            };
        };
        Self {
            pause_comments: candidate.pause_comments,
            pause_replies: candidate.pause_replies,
            pause_views: candidate.pause_views,
            pause_reactions: candidate.pause_reactions,
            pause_anon_views: candidate.pause_anon_views,
        }
    }

    #[must_use]
    pub const fn pause_comments(self) -> bool {
        self.pause_comments
    }

    #[must_use]
    pub const fn pause_replies(self) -> bool {
        self.pause_replies
    }

    #[must_use]
    pub const fn pause_views(self) -> bool {
        self.pause_views
    }

    #[must_use]
    pub const fn pause_reactions(self) -> bool {
        self.pause_reactions
    }

    #[must_use]
    pub const fn pause_anon_views(self) -> bool {
        self.pause_anon_views
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_invalidated_source_defaults_every_flag_together() {
        assert_eq!(
            LegacyNotificationPreferencesV1::from_validated_source(None),
            LegacyNotificationPreferencesV1::default()
        );
    }

    #[test]
    fn validated_source_preserves_every_boolean_without_policy_inference() {
        let preferences = LegacyNotificationPreferencesV1::from_validated_source(Some(
            LegacyNotificationPreferencesCandidateV1::new(true, false, true, false, true),
        ));
        assert!(preferences.pause_comments());
        assert!(!preferences.pause_replies());
        assert!(preferences.pause_views());
        assert!(!preferences.pause_reactions());
        assert!(preferences.pause_anon_views());
    }
}
