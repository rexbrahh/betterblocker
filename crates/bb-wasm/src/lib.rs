//! WebAssembly bindings for BetterBlocker

use std::sync::OnceLock;
use wasm_bindgen::prelude::*;
use bb_core::{
    Matcher,
    Snapshot,
    types::{MatchDecision, RequestContext, RequestType, SchemeMask},
    psl::get_etld1,
    url::extract_host,
};

struct MatcherState {
    #[allow(dead_code)]
    data: &'static [u8],
    #[allow(dead_code)]
    snapshot: &'static Snapshot<'static>,
    matcher: &'static Matcher<'static>,
}

static MATCHER_STATE: OnceLock<MatcherState> = OnceLock::new();

#[wasm_bindgen]
pub fn init(snapshot_data: &[u8]) -> Result<(), JsValue> {
    if MATCHER_STATE.get().is_some() {
        return Err(JsValue::from_str("Already initialized. Reload the page to reinitialize."));
    }

    let data: &'static [u8] = Box::leak(snapshot_data.to_vec().into_boxed_slice());
    
    let snapshot: &'static Snapshot<'static> = Box::leak(Box::new(
        Snapshot::load(data)
            .map_err(|e| JsValue::from_str(&format!("Failed to load snapshot: {}", e)))?
    ));
    
    let matcher: &'static Matcher<'static> = Box::leak(Box::new(Matcher::new(snapshot)));
    
    MATCHER_STATE.set(MatcherState { data, snapshot, matcher })
        .map_err(|_| JsValue::from_str("Failed to set matcher state"))?;
    
    Ok(())
}

#[wasm_bindgen]
pub fn is_initialized() -> bool {
    MATCHER_STATE.get().is_some()
}

#[wasm_bindgen]
pub fn get_snapshot_info() -> JsValue {
    let result = js_sys::Object::new();
    if let Some(state) = MATCHER_STATE.get() {
        let _ = js_sys::Reflect::set(&result, &"size".into(), &JsValue::from(state.data.len()));
        let _ = js_sys::Reflect::set(&result, &"initialized".into(), &JsValue::from(true));
    } else {
        let _ = js_sys::Reflect::set(&result, &"initialized".into(), &JsValue::from(false));
    }
    result.into()
}

#[wasm_bindgen]
pub fn match_request(
    url: &str,
    request_type: &str,
    initiator: Option<String>,
    tab_id: i32,
    frame_id: i32,
    request_id: &str,
) -> JsValue {
    let matcher = match MATCHER_STATE.get() {
        Some(state) => state.matcher,
        None => {
            let result = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&result, &"decision".into(), &JsValue::from(0));
            let _ = js_sys::Reflect::set(&result, &"ruleId".into(), &JsValue::from(-1));
            let _ = js_sys::Reflect::set(&result, &"listId".into(), &JsValue::from(0));
            return result.into();
        }
    };

    let req_host = extract_host(url).unwrap_or("");
    let req_etld1 = get_etld1(req_host);
    
    let (site_host, site_etld1) = match &initiator {
        Some(init) if !init.is_empty() => {
            let h = extract_host(init).unwrap_or("");
            (h, get_etld1(h))
        }
        _ if request_type == "main_frame" => (req_host, req_etld1.clone()),
        _ => ("", String::new()),
    };
    
    let scheme = bb_core::url::extract_scheme(url).unwrap_or(SchemeMask::HTTP);
    let is_third_party = !site_etld1.is_empty() && req_etld1 != site_etld1;
    let request_type_mask = parse_request_type(request_type);
    
    let ctx = RequestContext {
        url,
        req_host,
        req_etld1: &req_etld1,
        site_host,
        site_etld1: &site_etld1,
        scheme,
        request_type: request_type_mask,
        is_third_party,
        tab_id,
        frame_id,
        request_id,
    };
    
    let result = matcher.match_request(&ctx);
    
    let js_result = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&js_result, &"decision".into(), &JsValue::from(result.decision as u8));
    let _ = js_sys::Reflect::set(&js_result, &"ruleId".into(), &JsValue::from(result.rule_id));
    let _ = js_sys::Reflect::set(&js_result, &"listId".into(), &JsValue::from(result.list_id));
    
    if let Some(redirect_url) = result.redirect_url {
        let _ = js_sys::Reflect::set(&js_result, &"redirectUrl".into(), &JsValue::from_str(&redirect_url));
    }
    
    js_result.into()
}

#[wasm_bindgen]
pub fn should_block(
    url: &str,
    request_type: &str,
    initiator: Option<String>,
) -> bool {
    let matcher = match MATCHER_STATE.get() {
        Some(state) => state.matcher,
        None => return false,
    };

    let req_host = extract_host(url).unwrap_or("");
    let req_etld1 = get_etld1(req_host);
    
    let (site_host, site_etld1) = match &initiator {
        Some(init) if !init.is_empty() => {
            let h = extract_host(init).unwrap_or("");
            (h, get_etld1(h))
        }
        _ => (req_host, req_etld1.clone()),
    };
    
    let scheme = bb_core::url::extract_scheme(url).unwrap_or(SchemeMask::HTTP);
    let is_third_party = !site_etld1.is_empty() && req_etld1 != site_etld1;
    let request_type_mask = parse_request_type(request_type);
    
    let ctx = RequestContext {
        url,
        req_host,
        req_etld1: &req_etld1,
        site_host,
        site_etld1: &site_etld1,
        scheme,
        request_type: request_type_mask,
        is_third_party,
        tab_id: -1,
        frame_id: -1,
        request_id: "",
    };
    
    matcher.match_request(&ctx).decision == MatchDecision::Block
}

#[wasm_bindgen]
pub fn get_etld1_js(host: &str) -> String {
    get_etld1(host)
}

#[wasm_bindgen]
pub fn is_same_site(host1: &str, host2: &str) -> bool {
    get_etld1(host1) == get_etld1(host2)
}

#[wasm_bindgen]
pub fn is_third_party_js(site_host: &str, req_host: &str) -> bool {
    get_etld1(site_host) != get_etld1(req_host)
}

#[wasm_bindgen]
pub fn extract_host_js(url: &str) -> Option<String> {
    extract_host(url).map(|h| h.to_string())
}

fn parse_request_type(request_type: &str) -> RequestType {
    match request_type {
        "main_frame" | "document" => RequestType::MAIN_FRAME,
        "sub_frame" | "subdocument" => RequestType::SUBDOCUMENT,
        "stylesheet" | "css" => RequestType::STYLESHEET,
        "script" | "js" => RequestType::SCRIPT,
        "image" | "img" => RequestType::IMAGE,
        "font" => RequestType::FONT,
        "object" => RequestType::OBJECT,
        "xmlhttprequest" | "xhr" => RequestType::XMLHTTPREQUEST,
        "ping" | "beacon" => RequestType::PING,
        "media" => RequestType::MEDIA,
        "websocket" | "ws" => RequestType::WEBSOCKET,
        "other" => RequestType::OTHER,
        _ => RequestType::OTHER,
    }
}
