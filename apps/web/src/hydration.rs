use leptos::prelude::*;

pub const ROOT_ID: &str = "frame-hydration-root";
pub const PLAYER_HELP_ROOT_ID: &str = "frame-player-help-root";
pub const PUBLIC_COLLABORATION_ROOT_ID: &str = "frame-public-collaboration-root";
pub const AUTHENTICATED_ROOT_ID: &str = "frame-authenticated-workspace-root";

/// Authenticated product data is loaded only after browser hydration. The
/// relative-path transport sends the host-only session cookie directly to the
/// Worker; this island never receives a credential from Render or SSR HTML.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn AuthenticatedWorkspacePanel() -> impl IntoView {
    use std::rc::Rc;

    use crate::browser_authenticated::{
        BrowserAction, BrowserActionEffectState, BrowserAuthenticatedClient, BrowserMutationInput,
        BrowserWorkspace, WasmSameOriginTransport,
    };

    let status = RwSignal::new("Checking your same-origin Frame session.".to_owned());
    let workspace = RwSignal::new(None::<BrowserWorkspace>);
    let busy = RwSignal::new(false);
    let logout_busy = RwSignal::new(false);
    let action_value = RwSignal::new(String::new());
    let uncertain_mutation = RwSignal::new(None::<BrowserMutationInput>);
    let route = current_authenticated_request();
    let client = Rc::new(BrowserAuthenticatedClient::new(WasmSameOriginTransport));

    if let Ok((surface, query)) = route.clone() {
        let client = Rc::clone(&client);
        Effect::new(move |_| {
            let client = Rc::clone(&client);
            let query = query.clone();
            status.set("Loading the authorized workspace.".into());
            wasm_bindgen_futures::spawn_local(async move {
                match client.load(surface, &query).await {
                    Ok(loaded) => {
                        if surface.action() == Some(BrowserAction::SetActiveOrganization) {
                            action_value.set(
                                loaded
                                    .organizations
                                    .iter()
                                    .find(|choice| choice.active)
                                    .or_else(|| loaded.organizations.first())
                                    .map(|choice| choice.id.clone())
                                    .unwrap_or_default(),
                            );
                        }
                        status.set("Workspace loaded.".into());
                        workspace.set(Some(loaded));
                    }
                    Err(error) => {
                        workspace.set(None);
                        status.set(browser_error_message(error).into());
                    }
                }
            });
        });
    } else {
        status.set("This authenticated route is unavailable.".into());
    }

    let action = route
        .as_ref()
        .ok()
        .and_then(|(surface, _)| surface.action());
    let logout_client = Rc::clone(&client);
    let logout = move |_| {
        if logout_busy.get_untracked() {
            return;
        }
        let client = Rc::clone(&logout_client);
        logout_busy.set(true);
        status.set("Revoking this browser session.".into());
        wasm_bindgen_futures::spawn_local(async move {
            match client.logout().await {
                Ok(()) | Err(crate::browser_authenticated::BrowserClientError::Unauthenticated) => {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().set_href("/login");
                    }
                }
                Err(error) => {
                    status.set(browser_error_message(error).into());
                    logout_busy.set(false);
                }
            }
        });
    };
    let submit_client = Rc::clone(&client);
    let submit_route = route.clone();
    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        if busy.get_untracked() {
            return;
        }
        let (Ok((surface, query)), Some(action)) = (submit_route.clone(), action) else {
            status.set("The action is unavailable.".into());
            return;
        };
        let input = if let Some(input) = uncertain_mutation.get_untracked() {
            input
        } else {
            let Some(current) = workspace.get_untracked() else {
                status.set("The action is unavailable.".into());
                return;
            };
            if !action.permitted_for(current.role) {
                status.set("Your workspace role does not permit this action.".into());
                return;
            }
            let Some(idempotency_key) = random_operation_id() else {
                status.set("A safe operation identifier is unavailable.".into());
                return;
            };
            let value = action_value.get_untracked().trim().to_owned();
            if action.requires_value() && value.is_empty() {
                status.set("Enter a value before submitting.".into());
                return;
            }
            BrowserMutationInput {
                action,
                expected_revision: current.revision,
                selection_revision: current.selection_revision,
                selection_context: current.selection_context.clone(),
                idempotency_key,
                value: action.requires_value().then_some(value),
                resource_id: (action == BrowserAction::CreateFolder)
                    .then(|| current.spaces.first().map(|space| space.id.clone()))
                    .flatten(),
            }
        };
        let client = Rc::clone(&submit_client);
        busy.set(true);
        status.set(if uncertain_mutation.get_untracked().is_some() {
            "Retrying the exact request through the authenticated Worker boundary.".into()
        } else {
            "Submitting through the authenticated Worker boundary.".into()
        });
        wasm_bindgen_futures::spawn_local(async move {
            match client.mutate(&input).await {
                Ok(receipt) => {
                    uncertain_mutation.set(None);
                    match client.load(surface, &query).await {
                        Ok(reloaded) => {
                            action_value.set(String::new());
                            workspace.set(Some(reloaded));
                            status.set(match receipt.effect_state {
                                BrowserActionEffectState::Applied => format!(
                                    "Action applied at revision {}. Authorized views were refreshed.",
                                    receipt.revision
                                ),
                                BrowserActionEffectState::PendingProtectedExecution => format!(
                                    "Request recorded at revision {} for protected execution. No provider change is claimed yet.",
                                    receipt.revision
                                ),
                            });
                        }
                        Err(error) => {
                            workspace.set(None);
                            status.set(browser_error_message(error).into());
                        }
                    }
                }
                Err(error) => {
                    let outcome_unknown =
                        error == crate::browser_authenticated::BrowserClientError::Unavailable;
                    if outcome_unknown {
                        if let Some(value) = &input.value {
                            action_value.set(value.clone());
                        }
                        uncertain_mutation.set(Some(input));
                    } else {
                        uncertain_mutation.set(None);
                    }
                    workspace.set(None);
                    let refreshed = client.load(surface, &query).await;
                    if let Ok(reloaded) = refreshed {
                        workspace.set(Some(reloaded));
                    }
                    if outcome_unknown {
                        status.set(
                            "The action outcome was not confirmed. Cached workspace data was discarded and authorized data was refreshed where possible. Submit again to retry the exact same request safely."
                                .into(),
                        );
                    } else {
                        status.set(browser_error_message(error).into());
                    }
                }
            }
            busy.set(false);
        });
    };

    view! {
        <section class="panel authenticated-browser-panel" aria-labelledby="browser-workspace-title">
            <p class="eyebrow">"Private workspace"</p>
            <h1 id="browser-workspace-title">"Frame workspace"</h1>
            <p role="status" aria-live="polite" aria-atomic="true">
                {move || status.get()}
            </p>
            <button
                class="button secondary"
                type="button"
                disabled=move || logout_busy.get()
                on:click=logout
            >
                {move || if logout_busy.get() { "Signing out…" } else { "Sign out" }}
            </button>
            {move || workspace.get().map(|current| {
                let recording_items = current.recordings.iter().map(|recording| {
                    view! { <li>{format!("{} · {}", recording.title, recording.state)}</li> }
                }).collect_view();
                let resource_summary = format!(
                    "{} spaces · {} folders",
                    current.spaces.len(),
                    current.folders.len(),
                );
                view! {
                    <div data-frame-authenticated-ready="true">
                        <h2>{current.organization_name}</h2>
                        <p>{format!("{} · {}", current.member_label, current.role.as_str())}</p>
                        <p>{resource_summary}</p>
                        <ul aria-label="Authorized recordings">{recording_items}</ul>
                    </div>
                }
            })}
            {action.map(|action| view! {
                <form
                    class="stack authenticated-action-form"
                    data-frame-action=action.as_str()
                    hidden=move || uncertain_mutation.get().is_none()
                        && !workspace.get().is_some_and(|current| {
                            action.permitted_for(current.role)
                        })
                    on:submit=submit
                >
                    <label
                        for="authenticated-organization-choice"
                        hidden={action != BrowserAction::SetActiveOrganization}
                    >
                        "Active organization"
                    </label>
                    <select
                        id="authenticated-organization-choice"
                        hidden={action != BrowserAction::SetActiveOrganization}
                        required={action == BrowserAction::SetActiveOrganization}
                        disabled=move || busy.get()
                            || uncertain_mutation.get().is_some()
                            || !workspace.get().is_some_and(|current| {
                                action.permitted_for(current.role)
                            })
                        prop:value=move || action_value.get()
                        on:change=move |event| action_value.set(event_target_value(&event))
                    >
                        {move || workspace.get().into_iter().flat_map(|current| {
                            current.organizations.into_iter().map(|choice| {
                                view! {
                                    <option value=choice.id selected=choice.active>
                                        {choice.name}
                                    </option>
                                }
                            }).collect::<Vec<_>>()
                        }).collect_view()}
                    </select>
                    {(action.requires_value()
                        && action != BrowserAction::SetActiveOrganization).then(|| view! {
                        <label for="authenticated-action-value">"Action value"</label>
                        <input
                            id="authenticated-action-value"
                            maxlength="120"
                            required
                            disabled=move || busy.get()
                                || uncertain_mutation.get().is_some()
                                || !workspace.get().is_some_and(|current| {
                                    action.permitted_for(current.role)
                                })
                            prop:value=move || action_value.get()
                            on:input=move |event| action_value.set(event_target_value(&event))
                        />
                    })}
                    <button
                        class="button"
                        type="submit"
                        disabled=move || busy.get()
                            || uncertain_mutation.get().is_none()
                                && !workspace.get().is_some_and(|current| {
                                    action.permitted_for(current.role)
                                })
                    >
                        {move || if busy.get() {
                            "Submitting…"
                        } else if uncertain_mutation.get().is_some() {
                            "Retry exact request"
                        } else {
                            "Submit authenticated request"
                        }}
                    </button>
                </form>
            })}
        </section>
    }
}

#[cfg(target_arch = "wasm32")]
fn current_authenticated_request() -> Result<
    (
        crate::browser_authenticated::BrowserSurface,
        crate::browser_authenticated::BrowserQuery,
    ),
    crate::browser_authenticated::BrowserClientError,
> {
    use crate::browser_authenticated::{BrowserClientError, BrowserQuery, BrowserSurface};

    let location = web_sys::window()
        .ok_or(BrowserClientError::Unavailable)?
        .location();
    let path = location
        .pathname()
        .map_err(|_| BrowserClientError::Unavailable)?;
    let (surface, resource_id) =
        BrowserSurface::from_path(&path).ok_or(BrowserClientError::Invalid)?;
    let search = location
        .search()
        .map_err(|_| BrowserClientError::Unavailable)?;
    let params =
        web_sys::UrlSearchParams::new_with_str(&search).map_err(|_| BrowserClientError::Invalid)?;
    let page = params
        .get("page")
        .map(|page| page.parse::<u16>().map_err(|_| BrowserClientError::Invalid))
        .transpose()?;
    let query = BrowserQuery::new(params.get("q"), params.get("filter"), page, resource_id)?;
    Ok((surface, query))
}

#[cfg(target_arch = "wasm32")]
const fn browser_error_message(
    error: crate::browser_authenticated::BrowserClientError,
) -> &'static str {
    use crate::browser_authenticated::BrowserClientError;

    match error {
        BrowserClientError::Unauthenticated => {
            "Sign in is required. No private workspace data was displayed."
        }
        BrowserClientError::Forbidden | BrowserClientError::NotFound => {
            "Your workspace role does not permit this view or action."
        }
        BrowserClientError::Invalid => "The request is invalid. Review the form and try again.",
        BrowserClientError::Conflict => {
            "The workspace changed. Refresh before resubmitting this action."
        }
        BrowserClientError::RateLimited => "Too many attempts. Wait before retrying.",
        BrowserClientError::Unavailable => "The workspace is temporarily unavailable.",
    }
}

/// A deliberately data-free hydration boundary shared by the native SSR
/// renderer and browser Wasm. Private state continues to come only from a
/// same-origin, server-authorized bootstrap; hydration never infers a session.
#[component]
pub fn HydrationBoundary() -> impl IntoView {
    let ready = RwSignal::new(false);
    Effect::new(move |_| ready.set(true));

    view! {
        <span
            id="frame-hydration-state"
            hidden
            aria-hidden="true"
            data-frame-island-version="1"
            data-frame-hydrated=move || ready.get().then_some("true")
        >
            {move || if ready.get() {
                "Interactive enhancements ready."
            } else {
                "Server-rendered content ready."
            }}
        </span>
    }
}

/// Progressive player help is the only custom state in the initial web Wasm
/// slice. Native video controls and static keyboard help remain available when
/// JavaScript or its bundle fails; hydration replaces that help with a compact
/// disclosure.
#[component]
pub fn PlayerKeyboardHelp() -> impl IntoView {
    let open = RwSignal::new(false);
    let hydrated = RwSignal::new(false);
    let allow_fullscreen = RwSignal::new(false);
    let allow_picture_in_picture = RwSignal::new(false);
    let status = RwSignal::new("Server-rendered player ready.");
    Effect::new(move |_| {
        hydrated.set(true);
        configure_player(allow_fullscreen, allow_picture_in_picture, status);
    });

    view! {
        <section
            class="player-keyboard-help"
            data-frame-island-version="1"
            aria-labelledby="player-keyboard-help-title"
            data-frame-enhanced=move || hydrated.get().then_some("true")
        >
            <h2 id="player-keyboard-help-title">"Keyboard playback help"</h2>
            <div
                class="player-controls hydration-only"
                role="group"
                aria-label="Playback controls"
            >
                <button
                    class="button secondary compact"
                    type="button"
                    aria-controls="frame-public-player"
                    on:click=move |_| player_toggle(status)
                >
                    "Play or pause"
                </button>
                <button
                    class="button secondary compact"
                    type="button"
                    aria-controls="frame-public-player"
                    on:click=move |_| player_seek(-10.0, status)
                >
                    "Back 10 seconds"
                </button>
                <button
                    class="button secondary compact"
                    type="button"
                    aria-controls="frame-public-player"
                    on:click=move |_| player_seek(10.0, status)
                >
                    "Forward 10 seconds"
                </button>
                <label for="frame-playback-rate">"Speed"</label>
                <select
                    id="frame-playback-rate"
                    aria-controls="frame-public-player"
                    on:change=move |event| {
                        player_set_rate(&event_target_value(&event), status);
                    }
                >
                    <option value="0.5">"0.5×"</option>
                    <option value="0.75">"0.75×"</option>
                    <option value="1" selected>"1×"</option>
                    <option value="1.25">"1.25×"</option>
                    <option value="1.5">"1.5×"</option>
                    <option value="2">"2×"</option>
                </select>
                <button
                    class="button secondary compact"
                    type="button"
                    aria-controls="frame-public-player"
                    disabled=move || !allow_fullscreen.get()
                    on:click=move |_| player_fullscreen(status)
                >
                    "Fullscreen"
                </button>
                <button
                    class="button secondary compact"
                    type="button"
                    aria-controls="frame-public-player"
                    disabled=move || !allow_picture_in_picture.get()
                    on:click=move |_| player_picture_in_picture(status)
                >
                    "Picture in picture"
                </button>
                <button
                    class="button secondary compact"
                    type="button"
                    aria-controls="frame-public-player"
                    on:click=move |_| player_retry(status)
                >
                    "Retry playback"
                </button>
            </div>
            <p
                class="hydration-only player-status"
                role="status"
                aria-live="polite"
                aria-atomic="true"
            >
                {move || status.get()}
            </p>
            <button
                class="button secondary compact hydration-only"
                type="button"
                aria-controls="player-keyboard-help-panel"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|value| *value = !*value)
            >
                {move || if open.get() { "Hide shortcuts" } else { "Show shortcuts" }}
            </button>
            <div
                id="player-keyboard-help-panel"
                class="hydration-only player-keyboard-help-panel"
                hidden=move || !open.get()
            >
                <p>
                    "Focus the native player controls. Use Space to play or pause, arrow keys to seek or adjust volume, and the captions control to select an available track."
                </p>
            </div>
            <p class="player-keyboard-help-fallback">
                "Focus the native player controls. Use Space to play or pause, arrow keys to seek or adjust volume, and the captions control to select an available track. Fullscreen and picture in picture remain subject to browser and share policy."
            </p>
        </section>
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct BrowserComment {
    id: String,
    body: String,
    timeline_ms: Option<u64>,
    state: String,
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct BrowserCommentList {
    comments: Vec<BrowserComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct BrowserTranscriptCue {
    start_ms: u64,
    end_ms: u64,
    speaker: Option<String>,
    text: String,
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct BrowserTranscript {
    language: String,
    segments: Vec<BrowserTranscriptCue>,
}

#[derive(Clone, PartialEq, Eq, serde::Deserialize)]
struct BrowserCollaborationGrant {
    token: String,
    expires_at_ms: u64,
    comments_enabled: bool,
    analytics_enabled: bool,
    analytics_policy_version: String,
}

impl std::fmt::Debug for BrowserCollaborationGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserCollaborationGrant")
            .field("token", &"<redacted>")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("comments_enabled", &self.comments_enabled)
            .field("analytics_enabled", &self.analytics_enabled)
            .field("analytics_policy_version", &self.analytics_policy_version)
            .finish()
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
struct BrowserConsent {
    granted: bool,
}

/// Progressive same-origin collaboration for a public share. The initial
/// server markup is useful and data-free; browser code reads only the validated
/// share capability attached to its root, keeps grants in memory, and renders
/// server-validated response DTOs as text.
#[component]
pub fn PublicCollaborationPanel() -> impl IntoView {
    let hydrated = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let comment_body = RwSignal::new(String::new());
    let comments = RwSignal::new(Vec::<BrowserComment>::new());
    let transcript = RwSignal::new(Vec::<BrowserTranscriptCue>::new());
    let transcript_language = RwSignal::new(None::<String>);
    let grant = RwSignal::new(None::<BrowserCollaborationGrant>);
    let analytics_allowed = RwSignal::new(false);
    let status = RwSignal::new(String::from(
        "Playback is ready. Interactive collaboration has not loaded yet.",
    ));

    Effect::new(move |_| {
        hydrated.set(true);
        load_public_collaboration(comments, transcript, transcript_language, status);
    });

    view! {
        <section
            class="public-collaboration"
            data-frame-island-version="1"
            data-frame-enhanced=move || hydrated.get().then_some("true")
            aria-labelledby="public-collaboration-title"
        >
            <h2 id="public-collaboration-title">"Interactive collaboration"</h2>
            <p
                class="collaboration-status"
                role="status"
                aria-live="polite"
                aria-atomic="true"
            >
                {move || status.get()}
            </p>
            <div class="hydration-only collaboration-grid">
                <section aria-labelledby="interactive-transcript-title">
                    <h3 id="interactive-transcript-title">"Transcript cues"</h3>
                    {move || {
                        let cues = transcript.get();
                        if cues.is_empty() {
                            view! { <p>"No interactive transcript is available."</p> }.into_any()
                        } else {
                            let language = transcript_language
                                .get()
                                .unwrap_or_else(|| "undetermined".into());
                            view! {
                                <p>{format!("Language: {language}")}</p>
                                <ol class="transcript-cues">
                                    {cues.into_iter().map(|cue| {
                                        let speaker = cue.speaker
                                            .map(|value| format!("{value}: "))
                                            .unwrap_or_default();
                                        view! {
                                            <li>
                                                <button
                                                    class="button secondary compact"
                                                    type="button"
                                                    on:click=move |_| player_seek_to(cue.start_ms, status)
                                                >
                                                    {format_collaboration_time(cue.start_ms)}
                                                </button>
                                                <span>{format!(" {speaker}{}", cue.text)}</span>
                                                <span class="visually-hidden">
                                                    {format!(" Ends at {}.", format_collaboration_time(cue.end_ms))}
                                                </span>
                                            </li>
                                        }
                                    }).collect_view()}
                                </ol>
                            }.into_any()
                        }
                    }}
                </section>
                <section aria-labelledby="interactive-comments-title">
                    <h3 id="interactive-comments-title">"Comments"</h3>
                    {move || {
                        let values = comments.get();
                        if values.is_empty() {
                            view! { <p>"No published comments are available."</p> }.into_any()
                        } else {
                            view! {
                                <ol class="public-comments">
                                    {values.into_iter().map(|comment| {
                                        let position = comment.timeline_ms.map(|milliseconds| {
                                            view! {
                                                <button
                                                    class="button secondary compact"
                                                    type="button"
                                                    on:click=move |_| player_seek_to(milliseconds, status)
                                                >
                                                    {format_collaboration_time(milliseconds)}
                                                </button>
                                            }
                                        });
                                        view! {
                                            <li data-comment-id=comment.id data-comment-state=comment.state>
                                                {position}
                                                <span>{comment.body}</span>
                                            </li>
                                        }
                                    }).collect_view()}
                                </ol>
                            }.into_any()
                        }
                    }}
                    <form
                        class="comment-form"
                        on:submit=move |event| {
                            event.prevent_default();
                            submit_public_comment(
                                comment_body.get_untracked(),
                                comments,
                                transcript,
                                transcript_language,
                                grant,
                                busy,
                                status,
                                comment_body,
                            );
                        }
                    >
                        <label for="frame-public-comment">"Add a comment"</label>
                        <textarea
                            id="frame-public-comment"
                            maxlength="4000"
                            rows="3"
                            prop:value=move || comment_body.get()
                            disabled=move || busy.get()
                            on:input=move |event| comment_body.set(event_target_value(&event))
                        ></textarea>
                        <button
                            class="button"
                            type="submit"
                            disabled=move || busy.get() || comment_body.get().trim().is_empty()
                        >
                            "Submit comment"
                        </button>
                    </form>
                </section>
                <section aria-labelledby="analytics-consent-title">
                    <h3 id="analytics-consent-title">"Playback analytics"</h3>
                    <p>"Choose explicitly. The decision and later events remain scoped to this share and in-memory grant."</p>
                    <div role="group" aria-label="Playback analytics consent">
                        <button
                            class="button secondary compact"
                            type="button"
                            disabled=move || busy.get()
                            on:click=move |_| set_public_analytics_consent(
                                true,
                                grant,
                                analytics_allowed,
                                busy,
                                status,
                            )
                        >
                            "Allow analytics"
                        </button>
                        <button
                            class="button secondary compact"
                            type="button"
                            disabled=move || busy.get()
                            on:click=move |_| set_public_analytics_consent(
                                false,
                                grant,
                                analytics_allowed,
                                busy,
                                status,
                            )
                        >
                            "Keep analytics off"
                        </button>
                    </div>
                </section>
            </div>
            <p class="collaboration-fallback">
                "Comments, transcript cues, and analytics choices require the optional same-origin interactive enhancement. Playback and caption downloads remain available without it."
            </p>
        </section>
    }
}

fn format_collaboration_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(any(target_arch = "wasm32", test))]
fn valid_collaboration_scope(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
        })
}

#[cfg(not(target_arch = "wasm32"))]
fn load_public_collaboration(
    comments: RwSignal<Vec<BrowserComment>>,
    transcript: RwSignal<Vec<BrowserTranscriptCue>>,
    transcript_language: RwSignal<Option<String>>,
    status: RwSignal<String>,
) {
    let _ = (comments, transcript, transcript_language, status);
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn submit_public_comment(
    body: String,
    comments: RwSignal<Vec<BrowserComment>>,
    transcript: RwSignal<Vec<BrowserTranscriptCue>>,
    transcript_language: RwSignal<Option<String>>,
    grant: RwSignal<Option<BrowserCollaborationGrant>>,
    busy: RwSignal<bool>,
    status: RwSignal<String>,
    comment_body: RwSignal<String>,
) {
    let _ = (
        body,
        comments,
        transcript,
        transcript_language,
        grant,
        busy,
        status,
        comment_body,
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn set_public_analytics_consent(
    granted: bool,
    grant: RwSignal<Option<BrowserCollaborationGrant>>,
    analytics_allowed: RwSignal<bool>,
    busy: RwSignal<bool>,
    status: RwSignal<String>,
) {
    let _ = (granted, grant, analytics_allowed, busy, status);
}

#[cfg(not(target_arch = "wasm32"))]
fn player_seek_to(milliseconds: u64, status: RwSignal<String>) {
    let _ = (milliseconds, status);
}

#[cfg(target_arch = "wasm32")]
fn collaboration_scope() -> Option<String> {
    let value = leptos::tachys::dom::document()
        .get_element_by_id(PUBLIC_COLLABORATION_ROOT_ID)?
        .get_attribute("data-frame-public-share")?;
    valid_collaboration_scope(&value).then_some(value)
}

#[cfg(target_arch = "wasm32")]
fn collaboration_path(scope: &str, suffix: &str) -> Option<String> {
    (valid_collaboration_scope(scope)
        && !suffix.is_empty()
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || matches!(byte, b'/' | b'-')))
    .then(|| format!("/api/v1/public/shares/{scope}/{suffix}"))
}

#[cfg(target_arch = "wasm32")]
fn load_public_collaboration(
    comments: RwSignal<Vec<BrowserComment>>,
    transcript: RwSignal<Vec<BrowserTranscriptCue>>,
    transcript_language: RwSignal<Option<String>>,
    status: RwSignal<String>,
) {
    let Some(scope) = collaboration_scope() else {
        status.set("Interactive collaboration requires a live share.".into());
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        refresh_public_collaboration(&scope, comments, transcript, transcript_language, status)
            .await;
    });
}

#[cfg(target_arch = "wasm32")]
async fn refresh_public_collaboration(
    scope: &str,
    comments: RwSignal<Vec<BrowserComment>>,
    transcript: RwSignal<Vec<BrowserTranscriptCue>>,
    transcript_language: RwSignal<Option<String>>,
    status: RwSignal<String>,
) {
    const MAX_RENDERED_TRANSCRIPT_CUES: usize = 500;

    status.set("Loading comments and transcript.".into());
    let comment_path = collaboration_path(scope, "comments");
    let transcript_path = collaboration_path(scope, "transcript");
    let comment_result = match comment_path {
        Some(path) => fetch_json::<BrowserCommentList>("GET", &path, None, None, None).await,
        None => Err(()),
    };
    let transcript_result = match transcript_path {
        Some(path) => fetch_json::<BrowserTranscript>("GET", &path, None, None, None).await,
        None => Err(()),
    };

    let mut loaded = false;
    if let Ok(list) = comment_result
        && list.comments.len() <= 200
    {
        comments.set(list.comments);
        loaded = true;
    }
    let mut transcript_truncated = false;
    if let Ok(document) = transcript_result {
        transcript_language.set(Some(document.language));
        transcript_truncated = document.segments.len() > MAX_RENDERED_TRANSCRIPT_CUES;
        transcript.set(
            document
                .segments
                .into_iter()
                .take(MAX_RENDERED_TRANSCRIPT_CUES)
                .collect(),
        );
        loaded = true;
    }
    status.set(if transcript_truncated {
        "Comments loaded. The interactive transcript is capped at 500 cues; use the caption download for the complete document."
            .into()
    } else if loaded {
        "Interactive collaboration loaded.".into()
    } else {
        "Interactive collaboration is unavailable. Playback remains available.".into()
    });
}

#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
fn submit_public_comment(
    body: String,
    comments: RwSignal<Vec<BrowserComment>>,
    transcript: RwSignal<Vec<BrowserTranscriptCue>>,
    transcript_language: RwSignal<Option<String>>,
    grant: RwSignal<Option<BrowserCollaborationGrant>>,
    busy: RwSignal<bool>,
    status: RwSignal<String>,
    comment_body: RwSignal<String>,
) {
    let body = body.trim().to_owned();
    if body.is_empty()
        || body.len() > 4_000
        || body
            .chars()
            .any(|value| value.is_control() && !matches!(value, '\n' | '\t'))
    {
        status.set("The comment must contain 1 to 4,000 safe characters.".into());
        return;
    }
    let Some(scope) = collaboration_scope() else {
        status.set("Comments are unavailable for this share.".into());
        return;
    };
    busy.set(true);
    status.set("Submitting comment.".into());
    wasm_bindgen_futures::spawn_local(async move {
        let outcome = async {
            let capability = ensure_collaboration_grant(&scope, grant).await?;
            if !capability.comments_enabled {
                return Err(());
            }
            let operation = random_operation_id().ok_or(())?;
            let path = collaboration_path(&scope, "comments").ok_or(())?;
            let payload = serde_json::to_string(&serde_json::json!({
                "idempotency_key": operation,
                "kind": "text",
                "body": body,
                "timeline_ms": player_position_ms(),
            }))
            .map_err(|_| ())?;
            fetch_json::<BrowserComment>(
                "POST",
                &path,
                Some(&payload),
                Some(&capability.token),
                Some(&operation),
            )
            .await
        }
        .await;
        match outcome {
            Ok(created) => {
                comment_body.set(String::new());
                if created.state == "published" {
                    comments.update(|values| {
                        if !values.iter().any(|value| value.id == created.id) {
                            values.push(created);
                            values.sort_by(|left, right| left.id.cmp(&right.id));
                        }
                    });
                    status.set("Comment published.".into());
                } else {
                    status.set("Comment submitted for moderation.".into());
                }
                refresh_public_collaboration(
                    &scope,
                    comments,
                    transcript,
                    transcript_language,
                    status,
                )
                .await;
            }
            Err(()) => status
                .set("The comment could not be submitted. Nothing was stored by the page.".into()),
        }
        busy.set(false);
    });
}

#[cfg(target_arch = "wasm32")]
fn set_public_analytics_consent(
    allowed: bool,
    grant: RwSignal<Option<BrowserCollaborationGrant>>,
    analytics_allowed: RwSignal<bool>,
    busy: RwSignal<bool>,
    status: RwSignal<String>,
) {
    let Some(scope) = collaboration_scope() else {
        status.set("Analytics consent is unavailable for this share.".into());
        return;
    };
    busy.set(true);
    analytics_allowed.set(false);
    status.set("Recording the analytics choice.".into());
    wasm_bindgen_futures::spawn_local(async move {
        let outcome = async {
            let capability = ensure_collaboration_grant(&scope, grant).await?;
            if !capability.analytics_enabled {
                return Err(());
            }
            let operation = random_operation_id().ok_or(())?;
            let path = collaboration_path(&scope, "analytics/consent").ok_or(())?;
            let payload = serde_json::to_string(&serde_json::json!({
                "idempotency_key": operation,
                "policy_version": capability.analytics_policy_version,
                "decision": if allowed { "grant" } else { "deny" },
            }))
            .map_err(|_| ())?;
            let consent = fetch_json::<BrowserConsent>(
                "PUT",
                &path,
                Some(&payload),
                Some(&capability.token),
                Some(&operation),
            )
            .await?;
            if consent.granted != allowed {
                return Err(());
            }
            Ok(capability)
        }
        .await;
        match outcome {
            Ok(capability) if allowed => {
                analytics_allowed.set(true);
                install_public_analytics(
                    &scope,
                    &capability.token,
                    &capability.analytics_policy_version,
                    analytics_allowed,
                );
                status.set("Playback analytics allowed for this share.".into());
            }
            Ok(_) => {
                analytics_allowed.set(false);
                status.set("Playback analytics remain off.".into());
            }
            Err(()) => {
                analytics_allowed.set(false);
                status.set(
                    "The analytics choice could not be recorded; analytics remain off.".into(),
                );
            }
        }
        busy.set(false);
    });
}

#[cfg(target_arch = "wasm32")]
async fn ensure_collaboration_grant(
    scope: &str,
    grant: RwSignal<Option<BrowserCollaborationGrant>>,
) -> Result<BrowserCollaborationGrant, ()> {
    let now = browser_now_ms();
    if let Some(existing) = grant.get_untracked()
        && existing.expires_at_ms > now.saturating_add(30_000)
    {
        return Ok(existing);
    }
    let path = collaboration_path(scope, "collaboration-grants").ok_or(())?;
    let created = fetch_json::<BrowserCollaborationGrant>("POST", &path, None, None, None).await?;
    if created.token.is_empty()
        || created.token.len() > 512
        || created.expires_at_ms <= now
        || created.analytics_policy_version.len() > 64
    {
        return Err(());
    }
    grant.set(Some(created.clone()));
    Ok(created)
}

#[cfg(target_arch = "wasm32")]
async fn fetch_json<T: serde::de::DeserializeOwned>(
    method: &str,
    path: &str,
    body: Option<&str>,
    token: Option<&str>,
    idempotency_key: Option<&str>,
) -> Result<T, ()> {
    use wasm_bindgen::{JsCast, JsValue};

    if !path.starts_with("/api/v1/public/shares/") || path.contains(['?', '#', '\\']) {
        return Err(());
    }
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    let headers = web_sys::Headers::new().map_err(|_| ())?;
    headers.set("accept", "application/json").map_err(|_| ())?;
    if let Some(body) = body {
        init.set_body(&JsValue::from_str(body));
        headers
            .set("content-type", "application/json")
            .map_err(|_| ())?;
    }
    if let Some(token) = token {
        headers
            .set("authorization", &format!("FrameShare {token}"))
            .map_err(|_| ())?;
    }
    if let Some(idempotency_key) = idempotency_key {
        headers
            .set("idempotency-key", idempotency_key)
            .map_err(|_| ())?;
    }
    init.set_headers(&headers);
    let request = web_sys::Request::new_with_str_and_init(path, &init).map_err(|_| ())?;
    let response = wasm_bindgen_futures::JsFuture::from(
        web_sys::window().ok_or(())?.fetch_with_request(&request),
    )
    .await
    .map_err(|_| ())?
    .dyn_into::<web_sys::Response>()
    .map_err(|_| ())?;
    if !response.ok() {
        return Err(());
    }
    let value = wasm_bindgen_futures::JsFuture::from(response.json().map_err(|_| ())?)
        .await
        .map_err(|_| ())?;
    serde_wasm_bindgen::from_value(value).map_err(|_| ())
}

#[cfg(target_arch = "wasm32")]
fn random_operation_id() -> Option<String> {
    let mut bytes = [0_u8; 16];
    let window = web_sys::window()?;
    let crypto = window.crypto().ok()?;
    crypto.get_random_values_with_u8_array(&mut bytes).ok()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Some(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    ))
}

#[cfg(target_arch = "wasm32")]
fn browser_now_ms() -> u64 {
    js_sys::Date::now().clamp(0.0, u64::MAX as f64) as u64
}

#[cfg(target_arch = "wasm32")]
fn player_position_ms() -> Option<u64> {
    player().and_then(|video| {
        let position = video.current_time();
        position
            .is_finite()
            .then(|| (position.max(0.0) * 1_000.0) as u64)
    })
}

#[cfg(target_arch = "wasm32")]
fn player_seek_to(milliseconds: u64, status: RwSignal<String>) {
    let Some(video) = player() else {
        status.set("Playback is unavailable.".into());
        return;
    };
    let requested = milliseconds as f64 / 1_000.0;
    let duration = video.duration();
    if !requested.is_finite() || (duration.is_finite() && duration >= 0.0 && requested > duration) {
        status.set("The requested transcript position is unavailable.".into());
        return;
    }
    video.set_current_time(requested);
    status.set(format!(
        "Moved playback to {}.",
        format_collaboration_time(milliseconds)
    ));
}

#[cfg(target_arch = "wasm32")]
fn install_public_analytics(
    scope: &str,
    token: &str,
    policy_version: &str,
    analytics_allowed: RwSignal<bool>,
) {
    use std::{cell::Cell, rc::Rc};

    use wasm_bindgen::{JsCast, closure::Closure};

    let document = leptos::tachys::dom::document();
    let Some(root) = document.get_element_by_id(PUBLIC_COLLABORATION_ROOT_ID) else {
        return;
    };
    if root
        .get_attribute("data-frame-analytics-listener")
        .as_deref()
        == Some("true")
    {
        return;
    }
    let Some(video) = player() else {
        return;
    };
    let sequence = Rc::new(Cell::new(0_u64));
    for (event_name, kind) in [
        ("play", "playback_started"),
        ("pause", "playback_paused"),
        ("ended", "playback_completed"),
        ("error", "playback_error"),
    ] {
        let scope = scope.to_owned();
        let token = token.to_owned();
        let policy_version = policy_version.to_owned();
        let sequence = Rc::clone(&sequence);
        let listener_video = video.clone();
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            if !analytics_allowed.get_untracked() {
                return;
            }
            let next = sequence.get().saturating_add(1);
            sequence.set(next);
            let Some(operation) = random_operation_id() else {
                return;
            };
            let Some(path) = collaboration_path(&scope, "analytics/events") else {
                return;
            };
            let position = listener_video.current_time();
            let position_ms = position
                .is_finite()
                .then(|| (position.max(0.0) * 1_000.0) as u64);
            let Ok(payload) = serde_json::to_string(&serde_json::json!({
                "idempotency_key": operation,
                "policy_version": policy_version,
                "sequence": next,
                "kind": kind,
                "position_ms": position_ms,
                "occurred_at_ms": browser_now_ms(),
            })) else {
                return;
            };
            let token = token.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = fetch_json::<serde_json::Value>(
                    "POST",
                    &path,
                    Some(&payload),
                    Some(&token),
                    Some(&operation),
                )
                .await;
            });
        });
        if video
            .add_event_listener_with_callback(event_name, listener.as_ref().unchecked_ref())
            .is_err()
        {
            return;
        }
        listener.forget();
    }
    let _ = root.set_attribute("data-frame-analytics-listener", "true");
}

#[cfg(not(target_arch = "wasm32"))]
fn configure_player(
    allow_fullscreen: RwSignal<bool>,
    allow_picture_in_picture: RwSignal<bool>,
    status: RwSignal<&'static str>,
) {
    let _ = (allow_fullscreen, allow_picture_in_picture, status);
}

#[cfg(target_arch = "wasm32")]
fn configure_player(
    allow_fullscreen: RwSignal<bool>,
    allow_picture_in_picture: RwSignal<bool>,
    status: RwSignal<&'static str>,
) {
    if let Some(video) = player() {
        allow_fullscreen.set(video.dataset().get("allowFullscreen").as_deref() == Some("true"));
        allow_picture_in_picture
            .set(video.dataset().get("allowPictureInPicture").as_deref() == Some("true"));
        status.set("Interactive player controls ready.");
    }
    install_embed_api(status);
}

#[cfg(not(target_arch = "wasm32"))]
fn player_toggle(status: RwSignal<&'static str>) {
    let _ = status;
}

#[cfg(target_arch = "wasm32")]
fn player_toggle(status: RwSignal<&'static str>) {
    let Some(video) = player() else {
        status.set("Playback is unavailable.");
        return;
    };
    if video.paused() {
        if video.play().is_ok() {
            status.set("Playback requested.");
        } else {
            status.set("Playback could not start. Use retry or the native controls.");
        }
    } else {
        if video.pause().is_ok() {
            status.set("Playback paused.");
        } else {
            status.set("Playback could not be paused.");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn player_seek(seconds: f64, status: RwSignal<&'static str>) {
    let _ = (seconds, status);
}

#[cfg(target_arch = "wasm32")]
fn player_seek(seconds: f64, status: RwSignal<&'static str>) {
    let Some(video) = player() else {
        status.set("Playback is unavailable.");
        return;
    };
    let duration = video.duration();
    let maximum = if duration.is_finite() && duration >= 0.0 {
        duration
    } else {
        crate::share_player::MAX_RECORDING_DURATION_MS as f64 / 1_000.0
    };
    video.set_current_time((video.current_time() + seconds).clamp(0.0, maximum));
    status.set(if seconds < 0.0 {
        "Moved back 10 seconds."
    } else {
        "Moved forward 10 seconds."
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn player_set_rate(value: &str, status: RwSignal<&'static str>) {
    let _ = (value, status);
}

#[cfg(target_arch = "wasm32")]
fn player_set_rate(value: &str, status: RwSignal<&'static str>) {
    let Ok(rate) = value.parse::<f64>() else {
        status.set("Playback speed was rejected.");
        return;
    };
    if ![0.5, 0.75, 1.0, 1.25, 1.5, 2.0].contains(&rate) {
        status.set("Playback speed was rejected.");
        return;
    }
    if let Some(video) = player() {
        video.set_playback_rate(rate);
        status.set("Playback speed changed.");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn player_fullscreen(status: RwSignal<&'static str>) {
    let _ = status;
}

#[cfg(target_arch = "wasm32")]
fn player_fullscreen(status: RwSignal<&'static str>) {
    let Some(video) = player() else {
        status.set("Playback is unavailable.");
        return;
    };
    if video.dataset().get("allowFullscreen").as_deref() != Some("true") {
        status.set("Fullscreen is disabled by share policy.");
        return;
    }
    if video.request_fullscreen().is_ok() {
        status.set("Fullscreen requested.");
    } else {
        status.set("Fullscreen is not available in this browser.");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn player_picture_in_picture(status: RwSignal<&'static str>) {
    let _ = status;
}

#[cfg(target_arch = "wasm32")]
fn player_picture_in_picture(status: RwSignal<&'static str>) {
    use wasm_bindgen::{JsCast, JsValue};

    let Some(video) = player() else {
        status.set("Playback is unavailable.");
        return;
    };
    if video.dataset().get("allowPictureInPicture").as_deref() != Some("true") {
        status.set("Picture in picture is disabled by share policy.");
        return;
    }
    let Ok(method) = js_sys::Reflect::get(
        video.as_ref(),
        &JsValue::from_str("requestPictureInPicture"),
    ) else {
        status.set("Picture in picture is not available in this browser.");
        return;
    };
    let Some(method) = method.dyn_ref::<js_sys::Function>() else {
        status.set("Picture in picture is not available in this browser.");
        return;
    };
    if method.call0(video.as_ref()).is_ok() {
        status.set("Picture in picture requested.");
    } else {
        status.set("Picture in picture could not start.");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn player_retry(status: RwSignal<&'static str>) {
    let _ = status;
}

#[cfg(target_arch = "wasm32")]
fn player_retry(status: RwSignal<&'static str>) {
    let Some(video) = player() else {
        status.set("Playback is unavailable.");
        return;
    };
    video.load();
    if video.play().is_ok() {
        status.set("Playback retry requested.");
    } else {
        status.set("Playback retry failed. Check the connection and native controls.");
    }
}

#[cfg(target_arch = "wasm32")]
fn player() -> Option<web_sys::HtmlVideoElement> {
    use wasm_bindgen::JsCast;

    leptos::tachys::dom::document()
        .get_element_by_id("frame-public-player")?
        .dyn_into()
        .ok()
}

#[cfg(target_arch = "wasm32")]
fn install_embed_api(status: RwSignal<&'static str>) {
    use std::{cell::RefCell, rc::Rc};

    use wasm_bindgen::{JsCast, JsValue, closure::Closure};

    use crate::share_player::{
        EMBED_COMMAND_SCHEMA, EmbedCommandEnvelope, EmbedPlayerState, EmbedReplyEnvelope,
        EmbedSession,
    };

    let document = leptos::tachys::dom::document();
    let Some(root) = document.get_element_by_id(PLAYER_HELP_ROOT_ID) else {
        return;
    };
    let Some(share_scope) = root.get_attribute("data-frame-embed-share") else {
        return;
    };
    let Some(origins) = root.get_attribute("data-frame-embed-origins") else {
        return;
    };
    if root.get_attribute("data-frame-embed-listener").as_deref() == Some("true") {
        return;
    }
    let origins = origins
        .split_ascii_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let Ok(session) = EmbedSession::new(&share_scope, origins) else {
        return;
    };
    let window = web_sys::window().expect("browser window exists during hydration");
    let Ok(Some(parent)) = window.parent() else {
        return;
    };
    let session = Rc::new(RefCell::new(session));
    let listener_share = share_scope.clone();
    let listener_parent = parent.clone();
    let listener =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let source = js_sys::Reflect::get(event.as_ref(), &JsValue::from_str("source"));
            let source_is_parent = source
                .as_ref()
                .is_ok_and(|source| js_sys::Object::is(source, listener_parent.as_ref()));
            let Ok(envelope) = serde_wasm_bindgen::from_value::<EmbedCommandEnvelope>(event.data())
            else {
                return;
            };
            if envelope.schema != EMBED_COMMAND_SCHEMA {
                return;
            }
            let sequence = envelope.sequence;
            let Ok(command) =
                session
                    .borrow_mut()
                    .accept(&event.origin(), source_is_parent, envelope)
            else {
                return;
            };
            apply_embed_command(&command, status);
            let state = player().map(|video| EmbedPlayerState {
                paused: video.paused(),
                position_ms: (video.current_time().max(0.0) * 1_000.0) as u64,
                playback_rate_basis_points: (video.playback_rate() * 10_000.0)
                    .clamp(5_000.0, 20_000.0) as u16,
            });
            let Some(reply) = EmbedReplyEnvelope::accepted(&listener_share, sequence, state) else {
                return;
            };
            let Ok(reply) = serde_wasm_bindgen::to_value(&reply) else {
                return;
            };
            let _ = listener_parent.post_message(&reply, &event.origin());
        });
    if window
        .add_event_listener_with_callback("message", listener.as_ref().unchecked_ref())
        .is_ok()
    {
        let _ = root.set_attribute("data-frame-embed-listener", "true");
        listener.forget();
    }
}

#[cfg(target_arch = "wasm32")]
fn apply_embed_command(
    command: &crate::share_player::EmbedCommand,
    status: RwSignal<&'static str>,
) {
    match command {
        crate::share_player::EmbedCommand::Play => player_toggle(status),
        crate::share_player::EmbedCommand::Pause => {
            if let Some(video) = player()
                && video.pause().is_ok()
            {
                status.set("Playback paused by the embedding page.");
            }
        }
        crate::share_player::EmbedCommand::Seek { position_ms } => {
            if let Some(video) = player() {
                video.set_current_time(*position_ms as f64 / 1_000.0);
                status.set("Playback position changed by the embedding page.");
            }
        }
        crate::share_player::EmbedCommand::SetPlaybackRate { basis_points } => {
            if let Some(video) = player() {
                video.set_playback_rate(f64::from(*basis_points) / 10_000.0);
                status.set("Playback speed changed by the embedding page.");
            }
        }
        crate::share_player::EmbedCommand::RequestState => {
            status.set("Player state shared with the embedding page.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hydration_boundary_has_stable_server_markup() {
        let html = view! { <HydrationBoundary/> }.to_html();
        assert!(html.contains("frame-hydration-state"));
        assert!(html.contains("data-frame-island-version=\"1\""));
        assert!(html.contains("Server-rendered content ready."));
        assert!(html.contains("aria-hidden=\"true\""));
        assert!(!html.contains("aria-live="));
        assert!(!html.contains("role=\"status\""));
        assert!(!html.contains("Interactive enhancements ready."));
        assert!(!html.contains("data-frame-hydrated=\"true\""));
    }

    #[test]
    fn player_help_keeps_a_static_degraded_mode_fallback() {
        let html = view! { <PlayerKeyboardHelp/> }.to_html();
        assert!(html.contains("player-keyboard-help-fallback"));
        assert!(!html.contains("<noscript>"));
        assert!(html.contains("Keyboard playback help"));
        assert!(html.contains("data-frame-island-version=\"1\""));
        assert!(html.contains("aria-controls=\"player-keyboard-help-panel\""));
        assert!(html.contains("aria-expanded=\"false\""));
        assert!(html.contains("aria-controls=\"frame-public-player\""));
        assert!(html.contains("Play or pause"));
        assert!(html.contains("Back 10 seconds"));
        assert!(html.contains("Forward 10 seconds"));
        assert!(html.contains("Retry playback"));
        assert!(html.contains("role=\"status\""));
        assert!(html.contains("aria-live=\"polite\""));
        assert!(html.contains(" hidden"));
        assert!(!html.contains("onclick="));
    }

    #[test]
    fn collaboration_panel_is_progressively_enhanced_and_consent_first() {
        let html = view! { <PublicCollaborationPanel/> }.to_html();
        assert!(html.contains("Interactive collaboration"));
        assert!(html.contains("collaboration-fallback"));
        assert!(html.contains("Add a comment"));
        assert!(html.contains("Allow analytics"));
        assert!(html.contains("Keep analytics off"));
        assert!(html.contains("maxlength=\"4000\""));
        assert!(html.contains("aria-live=\"polite\""));
        assert!(!html.contains("FrameShare"));
        assert!(!html.contains("authorization"));
        assert!(!html.contains("data-frame-enhanced=\"true\""));
    }

    #[test]
    fn collaboration_scope_requires_a_lowercase_uuid() {
        assert!(valid_collaboration_scope(
            "018f47a6-7b1c-7f55-8f39-8f8a8690c501"
        ));
        assert!(!valid_collaboration_scope("fixture-public"));
        assert!(!valid_collaboration_scope(
            "018F47A6-7B1C-7F55-8F39-8F8A8690C501"
        ));
        assert!(!valid_collaboration_scope(
            "018f47a6-7b1c-7f55-8f39-8f8a8690c50/"
        ));
    }
}
