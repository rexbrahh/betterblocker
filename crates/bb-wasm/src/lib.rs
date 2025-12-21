//! WebAssembly bindings for BetterBlocker

use std::sync::OnceLock;
use wasm_bindgen::prelude::*;
use bb_compiler::{build_snapshot, optimize_rules, parse_filter_list};
use bb_core::{
    Matcher,
    Snapshot,
    matcher::ResponseHeader,
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
pub fn compile_filter_lists(list_texts: JsValue) -> Result<JsValue, JsValue> {
    let list_array = js_sys::Array::from(&list_texts);
    let list_count = list_array.length() as usize;

    if list_count == 0 {
        return Err(JsValue::from_str("No list texts provided"));
    }

    let mut all_rules = Vec::new();
    let mut line_counts: Vec<usize> = Vec::with_capacity(list_count);
    let mut rules_before_per_list: Vec<usize> = Vec::with_capacity(list_count);

    for (idx, value) in list_array.iter().enumerate() {
        let text = value
            .as_string()
            .ok_or_else(|| JsValue::from_str("List text must be a string"))?;

        line_counts.push(text.lines().count());

        let mut rules = parse_filter_list(&text);
        for rule in &mut rules {
            rule.list_id = idx as u16;
        }

        rules_before_per_list.push(rules.len());
        all_rules.extend(rules);
    }

    let optimize_stats = optimize_rules(&mut all_rules);
    let rules_before_total = optimize_stats.before;
    let rules_after_total = optimize_stats.after;

    let mut rules_after_per_list = vec![0usize; list_count];
    for rule in &all_rules {
        let list_id = rule.list_id as usize;
        if list_id < list_count {
            rules_after_per_list[list_id] += 1;
        }
    }

    let snapshot = build_snapshot(&all_rules);
    let js_result = js_sys::Object::new();
    let snapshot_array = js_sys::Uint8Array::from(snapshot.as_slice());

    let _ = js_sys::Reflect::set(&js_result, &"snapshot".into(), &snapshot_array);
    let _ = js_sys::Reflect::set(&js_result, &"rulesBefore".into(), &JsValue::from(rules_before_total as u32));
    let _ = js_sys::Reflect::set(&js_result, &"rulesAfter".into(), &JsValue::from(rules_after_total as u32));
    let _ = js_sys::Reflect::set(&js_result, &"rulesDeduped".into(), &JsValue::from(optimize_stats.deduped as u32));
    let _ = js_sys::Reflect::set(&js_result, &"badfilterRules".into(), &JsValue::from(optimize_stats.badfilter_rules as u32));
    let _ = js_sys::Reflect::set(&js_result, &"badfilteredRules".into(), &JsValue::from(optimize_stats.badfiltered_rules as u32));

    let list_stats = js_sys::Array::new_with_length(list_count as u32);
    for i in 0..list_count {
        let stat = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&stat, &"lines".into(), &JsValue::from(line_counts[i] as u32));
        let _ = js_sys::Reflect::set(&stat, &"rulesBefore".into(), &JsValue::from(rules_before_per_list[i] as u32));
        let _ = js_sys::Reflect::set(&stat, &"rulesAfter".into(), &JsValue::from(rules_after_per_list[i] as u32));
        list_stats.set(i as u32, stat.into());
    }

    let _ = js_sys::Reflect::set(&js_result, &"listStats".into(), &list_stats);

    Ok(js_result.into())
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

    let is_main_frame = matches!(request_type, "main_frame" | "document");
    let site_host = if is_main_frame {
        req_host
    } else {
        initiator
            .as_deref()
            .and_then(extract_host)
            .filter(|host| !host.is_empty())
            .unwrap_or(req_host)
    };
    let site_etld1 = get_etld1(site_host);

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
pub fn match_response_headers(
    url: &str,
    request_type: &str,
    initiator: Option<String>,
    tab_id: i32,
    frame_id: i32,
    request_id: &str,
    headers: JsValue,
) -> JsValue {
    let matcher = match MATCHER_STATE.get() {
        Some(state) => state.matcher,
        None => {
            let result = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&result, &"cancel".into(), &JsValue::from(false));
            return result.into();
        }
    };

    let req_host = extract_host(url).unwrap_or("");
    let req_etld1 = get_etld1(req_host);

    let is_main_frame = matches!(request_type, "main_frame" | "document");
    let site_host = if is_main_frame {
        req_host
    } else {
        initiator
            .as_deref()
            .and_then(extract_host)
            .filter(|host| !host.is_empty())
            .unwrap_or(req_host)
    };
    let site_etld1 = get_etld1(site_host);

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

    let headers_array = js_sys::Array::from(&headers);
    let mut header_storage: Vec<(String, String)> = Vec::new();
    header_storage.reserve(headers_array.length() as usize);

    for entry in headers_array.iter() {
        let name = js_sys::Reflect::get(&entry, &"name".into())
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let value = js_sys::Reflect::get(&entry, &"value".into())
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default();
        header_storage.push((name, value));
    }

    let mut header_views: Vec<ResponseHeader<'_>> = Vec::with_capacity(header_storage.len());
    for (name, value) in &header_storage {
        header_views.push(ResponseHeader { name, value });
    }

    let result = matcher.match_response_headers(&ctx, &header_views);

    let js_result = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&js_result, &"cancel".into(), &JsValue::from(result.cancel));
    let _ = js_sys::Reflect::set(&js_result, &"ruleId".into(), &JsValue::from(result.rule_id));
    let _ = js_sys::Reflect::set(&js_result, &"listId".into(), &JsValue::from(result.list_id));

    if !result.csp_injections.is_empty() {
        let csp_array = js_sys::Array::new();
        for value in result.csp_injections {
            csp_array.push(&JsValue::from_str(&value));
        }
        let _ = js_sys::Reflect::set(&js_result, &"csp".into(), &csp_array);
    }

    if !result.remove_headers.is_empty() {
        let remove_array = js_sys::Array::new();
        for value in result.remove_headers {
            remove_array.push(&JsValue::from_str(&value));
        }
        let _ = js_sys::Reflect::set(&js_result, &"removeHeaders".into(), &remove_array);
    }

    js_result.into()
}

#[wasm_bindgen]
pub fn match_cosmetics(
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
            let _ = js_sys::Reflect::set(&result, &"css".into(), &JsValue::from(""));
            let _ = js_sys::Reflect::set(&result, &"enableGeneric".into(), &JsValue::from(true));
            let _ = js_sys::Reflect::set(&result, &"procedural".into(), &js_sys::Array::new());
            let _ = js_sys::Reflect::set(&result, &"scriptlets".into(), &js_sys::Array::new());
            return result.into();
        }
    };

    let req_host = extract_host(url).unwrap_or("");
    let req_etld1 = get_etld1(req_host);

    let is_main_frame = matches!(request_type, "main_frame" | "document");
    let site_host = if is_main_frame {
        req_host
    } else {
        initiator
            .as_deref()
            .and_then(extract_host)
            .filter(|host| !host.is_empty())
            .unwrap_or(req_host)
    };
    let site_etld1 = get_etld1(site_host);

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

    let result = matcher.match_cosmetics(&ctx);
    let js_result = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&js_result, &"css".into(), &JsValue::from_str(&result.css));
    let _ = js_sys::Reflect::set(&js_result, &"enableGeneric".into(), &JsValue::from(result.enable_generic));

    let procedural = js_sys::Array::new();
    let _ = js_sys::Reflect::set(&js_result, &"procedural".into(), &procedural);

    let scriptlets = js_sys::Array::new();
    for call in result.scriptlets {
        let call_obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&call_obj, &"name".into(), &JsValue::from_str(&call.name));
        let args_array = js_sys::Array::new();
        for arg in call.args {
            args_array.push(&JsValue::from_str(&arg));
        }
        let _ = js_sys::Reflect::set(&call_obj, &"args".into(), &args_array);
        scriptlets.push(&call_obj);
    }
    let _ = js_sys::Reflect::set(&js_result, &"scriptlets".into(), &scriptlets);

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

    let is_main_frame = matches!(request_type, "main_frame" | "document");
    let site_host = if is_main_frame {
        req_host
    } else {
        initiator
            .as_deref()
            .and_then(extract_host)
            .filter(|host| !host.is_empty())
            .unwrap_or(req_host)
    };
    let site_etld1 = get_etld1(site_host);

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
        "ping" => RequestType::PING,
        "beacon" => RequestType::BEACON,
        "fetch" => RequestType::FETCH,
        "csp_report" => RequestType::CSP_REPORT,
        "speculative" => RequestType::SPECULATIVE,
        "media" => RequestType::MEDIA,
        "websocket" | "ws" => RequestType::WEBSOCKET,
        "other" => RequestType::OTHER,
        _ => RequestType::OTHER,
    }
}
