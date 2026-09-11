use wasm_bindgen::{JsCast, prelude::*};

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
    let reply = serde_json::from_str(request)
        .map_err(|e| e.to_string())
        .and_then(crate::compute::run);
    serde_json::to_string(&reply).expect("compute reply serializes")
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
