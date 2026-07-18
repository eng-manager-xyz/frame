use std::fmt::Write as _;

use frame_media::{
    DiagnosticIssue, FactoryRequirement, MEDIA_APPLICATION_VERSION, MINIMUM_GSTREAMER_VERSION_TEXT,
    PlatformScope, RuntimeCapability, diagnose_runtime,
};

fn main() {
    let diagnostics = diagnose_runtime();
    let mut output = String::from("{");
    write!(
        output,
        "\"schema_version\":2,\"application_version\":{},\"manifest_version\":{},\"minimum_gstreamer\":{},\"ready\":{},",
        json_string(MEDIA_APPLICATION_VERSION),
        diagnostics.manifest_version,
        json_string(MINIMUM_GSTREAMER_VERSION_TEXT),
        diagnostics.is_ready()
    )
    .expect("write JSON");
    output.push_str("\"runtime_version\":");
    match &diagnostics.runtime_version {
        Some(version) => output.push_str(&json_string(version)),
        None => output.push_str("null"),
    }
    output.push_str(",\"issues\":[");
    for (index, issue) in diagnostics.issues.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&issue_json(issue));
    }
    output.push_str("],\"factories\":[");
    for (index, factory) in diagnostics.factories.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"factory\":{},\"capability\":{},\"requirement\":{},\"platform\":{},\"available\":{},\"trusted_provenance\":{},\"plugin_version\":{}}}",
            json_string(factory.factory),
            json_string(&capability(factory.capability)),
            json_string(requirement(factory.requirement)),
            json_string(platform(factory.platform)),
            factory.available,
            factory.trusted_provenance,
            factory
                .plugin_version
                .as_deref()
                .map_or_else(|| "null".to_owned(), json_string)
        )
        .expect("write JSON");
    }
    output.push_str("]}");
    println!("{output}");
    if !diagnostics.is_ready() {
        std::process::exit(2);
    }
}

fn issue_json(issue: &DiagnosticIssue) -> String {
    let (code, subject) = match issue {
        DiagnosticIssue::InitializationFailed => ("initialization_failed", None),
        DiagnosticIssue::RuntimeTooOld { required, .. } => ("runtime_too_old", Some(*required)),
        DiagnosticIssue::MissingRequiredFactory(factory) => {
            ("missing_required_factory", Some(*factory))
        }
        DiagnosticIssue::ProhibitedFactoryPresent(factory) => {
            ("prohibited_factory_present", Some(*factory))
        }
        DiagnosticIssue::PluginSearchPathOverride(variable) => {
            ("plugin_search_path_override", Some(*variable))
        }
        DiagnosticIssue::LoaderEnvironmentOverride(variable) => {
            ("loader_environment_override", Some(*variable))
        }
        DiagnosticIssue::TrustedPluginPathRequired(variable) => {
            ("trusted_plugin_path_required", Some(*variable))
        }
        DiagnosticIssue::FactoryOutsideTrustedRoot(factory) => {
            ("factory_outside_trusted_root", Some(*factory))
        }
    };
    match subject {
        Some(subject) => format!(
            "{{\"code\":{},\"subject\":{}}}",
            json_string(code),
            json_string(subject)
        ),
        None => format!("{{\"code\":{}}}", json_string(code)),
    }
}

const fn requirement(value: FactoryRequirement) -> &'static str {
    match value {
        FactoryRequirement::Required => "required",
        FactoryRequirement::Optional => "optional",
        FactoryRequirement::Prohibited => "prohibited",
    }
}

const fn platform(value: PlatformScope) -> &'static str {
    match value {
        PlatformScope::All => "all",
        PlatformScope::NativeDesktop => "native_desktop",
        PlatformScope::Linux => "linux",
        PlatformScope::MacOs => "macos",
        PlatformScope::Windows => "windows",
    }
}

fn capability(value: RuntimeCapability) -> String {
    value.to_string()
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => output.push('?'),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}
