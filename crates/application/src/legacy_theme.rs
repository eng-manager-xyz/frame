//! Audited compatibility contract for Cap's dashboard theme server action.
//!
//! The pinned action replaces one response cookie and resolves with `void`.
//! It has no durable business mutation and accepted no client idempotency key;
//! retries are therefore an intentional last-write-wins cookie replacement.

use std::fmt;

use thiserror::Error;

pub const LEGACY_WEB_THEME_OPERATION_ID: &str = "cap-v1-7773d3e70d1d5919";
pub const LEGACY_WEB_THEME_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/_components/actions.ts#setTheme";
pub const LEGACY_WEB_THEME_COOKIE_NAME: &str = "theme";
pub const LEGACY_WEB_THEME_COOKIE_PATH: &str = "/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyThemeSourcePinV1 {
    pub path: &'static str,
    pub sha256: &'static str,
}

pub const LEGACY_WEB_THEME_SOURCES: &[LegacyThemeSourcePinV1] = &[
    LegacyThemeSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/_components/actions.ts",
        sha256: "3f73b91e1e105555846014adfbc7498d5c719b536b5edcd8a3876167ed84ad1a",
    },
    LegacyThemeSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/layout.tsx",
        sha256: "65221996b10ee679b5868e6cea9002256fc8043eb95b2dbf6b56a222ce9c1d33",
    },
    LegacyThemeSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/Contexts.tsx",
        sha256: "2b2df78eab9dbdde2918a4b5993a31cf73fb8de2465d318c7b27d3d439f2a0cb",
    },
    LegacyThemeSourcePinV1 {
        path: "apps/web/package.json",
        sha256: "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
    },
    LegacyThemeSourcePinV1 {
        path: "pnpm-lock.yaml",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyThemeV1 {
    Light,
    Dark,
}

impl LegacyThemeV1 {
    pub fn parse(value: &str) -> Result<Self, LegacyThemeErrorV1> {
        match value {
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(LegacyThemeErrorV1::Invalid),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyThemeRetryV1 {
    LastWriteWinsWithoutClientIdempotency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyThemeProfileV1 {
    pub operation_id: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub legacy_identity: &'static str,
    pub sources: &'static [LegacyThemeSourcePinV1],
    pub authentication: &'static str,
    pub output: &'static str,
    pub retry: LegacyThemeRetryV1,
}

pub const LEGACY_WEB_THEME_PROFILE: LegacyThemeProfileV1 = LegacyThemeProfileV1 {
    operation_id: LEGACY_WEB_THEME_OPERATION_ID,
    kind: "server_action",
    method: "ACTION",
    legacy_identity: LEGACY_WEB_THEME_IDENTITY,
    sources: LEGACY_WEB_THEME_SOURCES,
    authentication: "session",
    output: "void_with_response_cookie",
    retry: LegacyThemeRetryV1::LastWriteWinsWithoutClientIdempotency,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyThemeCookieEffectV1 {
    pub name: &'static str,
    pub value: LegacyThemeV1,
    pub path: &'static str,
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyThemeErrorV1 {
    #[error("invalid theme")]
    Invalid,
}

impl fmt::Debug for LegacyThemeErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Invalid")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyThemeAdapterV1;

impl LegacyThemeAdapterV1 {
    #[must_use]
    pub const fn profile(self) -> &'static LegacyThemeProfileV1 {
        &LEGACY_WEB_THEME_PROFILE
    }

    pub fn execute(self, value: &str) -> Result<LegacyThemeCookieEffectV1, LegacyThemeErrorV1> {
        Ok(LegacyThemeCookieEffectV1 {
            name: LEGACY_WEB_THEME_COOKIE_NAME,
            value: LegacyThemeV1::parse(value)?,
            path: LEGACY_WEB_THEME_COOKIE_PATH,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_freezes_the_exact_action_and_source_closure() {
        let profile = LegacyThemeAdapterV1.profile();
        assert_eq!(profile.operation_id, "cap-v1-7773d3e70d1d5919");
        assert_eq!(profile.kind, "server_action");
        assert_eq!(profile.method, "ACTION");
        assert_eq!(profile.legacy_identity, LEGACY_WEB_THEME_IDENTITY);
        assert_eq!(profile.sources.len(), 5);
        assert_eq!(profile.authentication, "session");
        assert_eq!(profile.output, "void_with_response_cookie");
        assert_eq!(
            profile.retry,
            LegacyThemeRetryV1::LastWriteWinsWithoutClientIdempotency
        );
    }

    #[test]
    fn only_the_two_pinned_theme_values_produce_the_cookie_effect() {
        for (value, theme) in [
            ("light", LegacyThemeV1::Light),
            ("dark", LegacyThemeV1::Dark),
        ] {
            assert_eq!(
                LegacyThemeAdapterV1.execute(value),
                Ok(LegacyThemeCookieEffectV1 {
                    name: "theme",
                    value: theme,
                    path: "/",
                })
            );
        }
        for value in ["", "system", "Light", " dark", "dark ", "dark\n"] {
            assert_eq!(
                LegacyThemeAdapterV1.execute(value),
                Err(LegacyThemeErrorV1::Invalid)
            );
        }
    }

    #[test]
    fn retries_are_repeatable_last_write_wins_replacements() {
        let adapter = LegacyThemeAdapterV1;
        let first = adapter.execute("dark").expect("dark effect");
        let replay = adapter.execute("dark").expect("repeat effect");
        let replacement = adapter.execute("light").expect("replacement effect");
        assert_eq!(first, replay);
        assert_eq!(replacement.value, LegacyThemeV1::Light);
        assert_eq!(replacement.name, first.name);
        assert_eq!(replacement.path, first.path);
    }
}
