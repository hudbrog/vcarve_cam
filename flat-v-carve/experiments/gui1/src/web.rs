use crate::compute::SceneMeta;
use std::cell::RefCell;
use wasm_bindgen::{JsCast, prelude::*};

thread_local! {
    /// Payload of the last successful compute, exposed as a WASM memory view
    /// for the worker to transfer without a JSON round trip.
    static PAYLOAD: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[wasm_bindgen]
pub async fn start(canvas_id: &str) -> Result<(), JsValue> {
    let canvas = web_sys::window()
        .ok_or("No window")?
        .document()
        .ok_or("No document")?
        .get_element_by_id(canvas_id)
        .ok_or("Missing canvas")?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions {
                depth_buffer: 24,
                ..Default::default()
            },
            Box::new(|cc| Ok(Box::new(crate::app::App::new(cc)))),
        )
        .await
}
#[wasm_bindgen]
pub fn compute(request: &str) -> String {
    let reply = serde_json::from_str::<crate::compute::Request>(request)
        .map_err(|e| e.to_string())
        .and_then(crate::compute::run);
    match reply {
        Ok((meta, payload)) => {
            PAYLOAD.with(|stored| *stored.borrow_mut() = payload);
            serde_json::to_string(&Ok::<SceneMeta, String>(meta)).expect("metadata serializes")
        }
        Err(error) => {
            PAYLOAD.with(|stored| stored.borrow_mut().clear());
            serde_json::to_string(&Err::<SceneMeta, String>(error)).expect("error serializes")
        }
    }
}

/// View of the retained payload. Copy it before the next compute call.
#[wasm_bindgen]
pub fn payload_view() -> Vec<u8> {
    PAYLOAD.with(|stored| stored.borrow().clone())
}
#[wasm_bindgen]
pub fn protocol() -> String {
    crate::compute::PROTOCOL.into()
}

#[wasm_bindgen]
pub fn validate_recovery(text: &str) -> Result<String, JsValue> {
    let draft = crate::state::Draft::recover(text).map_err(|e| JsValue::from_str(&e))?;
    serde_json::to_string(&draft).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[wasm_bindgen]
pub fn validate_session(text: &str) -> Result<String, JsValue> {
    let record = crate::recovery::Stored::decode(text).map_err(|e| JsValue::from_str(&e))?;
    serde_json::to_string(&record).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Read-only snapshot of the running application for the browser input probe.
#[wasm_bindgen]
pub fn probe_state() -> String {
    crate::app::probe::snapshot()
}
