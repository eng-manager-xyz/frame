#[cfg(all(target_arch = "wasm32", feature = "csr"))]
mod browser {
    use frame_desktop_core::{RecorderAdapterState, ShellCapabilities};
    use js_sys::Reflect;
    use leptos::prelude::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::spawn_local;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(
            catch,
            js_namespace = ["window", "__TAURI__", "core"],
            js_name = invoke
        )]
        async fn invoke_without_args(command: &str) -> Result<JsValue, JsValue>;
    }

    async fn bootstrap_desktop() -> Result<ShellCapabilities, ()> {
        let tauri =
            Reflect::get(&js_sys::global(), &JsValue::from_str("__TAURI__")).map_err(|_| ())?;
        if tauri.is_null() || tauri.is_undefined() {
            return Err(());
        }
        let value = invoke_without_args("bootstrap_main")
            .await
            .map_err(|_| ())?;
        let capabilities: ShellCapabilities =
            serde_wasm_bindgen::from_value(value).map_err(|_| ())?;
        capabilities
            .is_current_backend_truth()
            .then_some(capabilities)
            .ok_or(())
    }

    #[component]
    fn App() -> impl IntoView {
        let status = RwSignal::new("Connecting to the native backend…".to_owned());
        Effect::new(move |_| {
            spawn_local(async move {
                let next = match bootstrap_desktop().await {
                    Ok(capabilities)
                        if capabilities.recorder_adapter == RecorderAdapterState::NotSelected =>
                    {
                        "Native shell ready. Capture adapter is not selected, so recording is disabled."
                            .to_owned()
                    }
                    Ok(_) => "Native backend ready.".to_owned(),
                    Err(_) => "Native backend unavailable. Recording remains disabled.".to_owned(),
                };
                status.set(next);
            });
        });

        view! {
            <header class="app-header">
                <p class="eyebrow">"Frame desktop"</p>
                <h1>"Record with backend truth"</h1>
                <p>"The shell never reports recording, saving, or export success before the native backend confirms it."</p>
            </header>
            <nav aria-label="Desktop workspace">
                <button type="button" aria-current="page">"Recorder"</button>
                <button type="button" disabled=true>"Editor"</button>
            </nav>
            <main id="main-content" tabindex="-1">
                <section aria-labelledby="recorder-heading">
                    <h2 id="recorder-heading">"Recorder"</h2>
                    <p>"Capture remains disabled until a platform adapter and permissions are confirmed."</p>
                    <button type="button" disabled=true aria-describedby="backend-status">
                        "Start recording"
                    </button>
                </section>
            </main>
            <p id="backend-status" class="status" role="status" aria-live="polite">
                {move || status.get()}
            </p>
        }
    }

    pub fn mount() {
        leptos::mount::mount_to_body(App);
    }
}

#[cfg(all(target_arch = "wasm32", feature = "csr"))]
fn main() {
    browser::mount();
}

#[cfg(not(all(target_arch = "wasm32", feature = "csr")))]
fn main() {}
