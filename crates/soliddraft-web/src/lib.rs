//! The web build of SolidDraft: the app compiled to WebAssembly and drawn into the page's
//! canvas. `trunk build` in this directory makes it; docs/web.md has the rest.
//!
//! The page opens on the start page. Add `?demo` to the page's address to start with the
//! sample part instead, `?preset=fast`, `studio` or `realistic` to start in that look,
//! and `?theme=dark` for the dark interface.

#![cfg(target_arch = "wasm32")]

use soliddraft_app::app::{Automation, SolidDraftApp};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

struct FeedbackLogger;

impl log::Log for FeedbackLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = format!("{}: {}", record.level(), record.args());
        match record.level() {
            log::Level::Error | log::Level::Warn => {
                let (code, level) = if record.level() == log::Level::Error {
                    ("app-error", "error")
                } else {
                    ("app-warning", "warning")
                };
                soliddraft_app::web::feedback_event(
                    serde_json::json!({"type": "diagnostic", "code": code, "level": level}),
                );
                if record.level() == log::Level::Error {
                    web_sys::console::error_1(&message.into());
                } else {
                    web_sys::console::warn_1(&message.into());
                }
            }
            _ => web_sys::console::log_1(&message.into()),
        }
    }

    fn flush(&self) {}
}

/// Starts the app when the page loads the module. The kernel worker loads the same
/// module and has no window; it only answers `kernel_worker_run`.
#[wasm_bindgen(start)]
pub fn start() {
    if web_sys::window().is_none() {
        return;
    }
    let _ = log::set_logger(&FeedbackLogger);
    log::set_max_level(log::LevelFilter::Info);
    std::panic::set_hook(Box::new(|info| {
        soliddraft_app::journal_browser::mark_crashed();
        let message = format!("SolidDraft stopped: {info}");
        web_sys::console::error_1(&message.clone().into());
        soliddraft_app::web::feedback_event(
            serde_json::json!({"type": "diagnostic", "code": "crash", "level": "error"}),
        );
        show_status(&message);
    }));
    let window = web_sys::window().expect("a browser window");
    let document = window.document().expect("a document");
    if let Some(root) = document.document_element() {
        let _ = root.set_attribute("data-feedback-version", env!("CARGO_PKG_VERSION"));
        let _ = root.set_attribute("data-feedback-commit", env!("SOLIDDRAFT_COMMIT"));
    }
    let canvas: web_sys::HtmlCanvasElement =
        document.get_element_by_id("soliddraft").expect("the page's canvas").unchecked_into();
    let query = window.location().search().unwrap_or_default();
    let demo = query.split(['?', '&']).any(|p| p == "demo");
    let preset = query
        .split(['?', '&'])
        .find_map(|p| p.strip_prefix("preset="))
        .and_then(soliddraft_app::viewport::look::Preset::parse);
    let theme =
        query.split(['?', '&']).find_map(|p| p.strip_prefix("theme=")).and_then(soliddraft_app::theme::Mode::parse);
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = soliddraft_app::journal_browser::start().await {
            show_status(&format!("SolidDraft could not start its command journal: {error}"));
            return;
        }
        let mut options = eframe::WebOptions::default();
        if software_webgpu().await
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup
        {
            // A software WebGPU adapter refuses buffers the viewport maps at creation, and
            // the app stops. The same machine draws through WebGL.
            log::info!("WebGPU runs in software here; drawing with WebGL");
            setup.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    let doc = if demo {
                        soliddraft_app::startup::demo_document()
                    } else {
                        soliddraft_app::startup::new_document()
                    };
                    let automation = Automation { connect_assistant: true, preset, theme, ..Default::default() };
                    let mut app = SolidDraftApp::new(cc, doc, automation);
                    // Unsaved work and the recent list stay in this browser's storage.
                    app.start_recovery_in_browser(soliddraft_app::settings::documents().autosave);
                    if !demo {
                        app.show_start_page(&cc.egui_ctx);
                    }
                    Ok(Box::new(app))
                }),
            )
            .await;
        match started {
            Ok(()) => hide_status(),
            Err(e) => show_status(&format!("SolidDraft could not start: {e:?}")),
        }
    });
}

/// True when the browser's WebGPU adapter is a software one, such as SwiftShader on a
/// machine without a usable GPU.
async fn software_webgpu() -> bool {
    let get = |object: &JsValue, key: &str| js_sys::Reflect::get(object, &key.into()).unwrap_or(JsValue::UNDEFINED);
    let gpu = get(&js_sys::global().unchecked_into::<web_sys::Window>().navigator().into(), "gpu");
    let Ok(request) = get(&gpu, "requestAdapter").dyn_into::<js_sys::Function>() else {
        return false;
    };
    let Ok(promise) = request.call0(&gpu).and_then(|promise| promise.dyn_into::<js_sys::Promise>()) else {
        return false;
    };
    let Ok(adapter) = wasm_bindgen_futures::JsFuture::from(promise).await else {
        return false;
    };
    if adapter.is_null() || adapter.is_undefined() {
        return false;
    }
    let info = get(&adapter, "info");
    get(&adapter, "isFallbackAdapter").is_truthy()
        || get(&info, "isFallbackAdapter").is_truthy()
        || get(&info, "architecture").as_string().is_some_and(|architecture| architecture == "swiftshader")
}

fn status() -> Option<web_sys::Element> {
    web_sys::window()?.document()?.get_element_by_id("status")
}

fn show_status(text: &str) {
    soliddraft_app::web::feedback_event(
        serde_json::json!({"type": "diagnostic", "code": "startup-error", "level": "error"}),
    );
    if let Some(el) = status() {
        el.set_text_content(Some(text));
        let _ = el.set_attribute("class", "shown");
    }
}

fn hide_status() {
    if let Some(el) = status() {
        let _ = el.set_attribute("class", "hidden");
    }
}
