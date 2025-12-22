//! WebAssembly bindings for BetterBlocker

use std::cell::RefCell;
use std::collections::HashMap;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum DynamicAction {
    Noop = 0,
    Block = 1,
    Allow = 2,
}

impl DynamicAction {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Block,
            2 => Self::Allow,
            _ => Self::Noop,
        }
    }
}

#[derive(Clone, Debug)]
struct DynamicRule {
    site: String,
    target: String,
    rule_type: String,
    action: DynamicAction,
}

struct RuntimeSettings {
    dynamic_filtering_enabled: bool,
    disabled_sites: Vec<String>,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            dynamic_filtering_enabled: true,
            disabled_sites: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct RemoveparamEntry {
    ts: u64,
    url: String,
}

#[derive(Clone, Debug)]
struct TraceEntry {
    url: String,
    request_type: String,
    initiator: Option<String>,
    tab_id: i32,
    frame_id: i32,
    request_id: String,
}

#[derive(Default)]
struct PerfBucket {
    values: Vec<f64>,
}

struct RuntimeState {
    dynamic_rules: Vec<DynamicRule>,
    settings: RuntimeSettings,
    removeparam_redirects: HashMap<String, RemoveparamEntry>,
    trace_enabled: bool,
    trace_max_entries: usize,
    trace_entries: Vec<TraceEntry>,
    perf_enabled: bool,
    perf_max_entries: usize,
    perf_before_request: PerfBucket,
    perf_headers_received: PerfBucket,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            dynamic_rules: Vec::new(),
            settings: RuntimeSettings::default(),
            removeparam_redirects: HashMap::new(),
            trace_enabled: false,
            trace_max_entries: MAX_TRACE_ENTRIES,
            trace_entries: Vec::new(),
            perf_enabled: false,
            perf_max_entries: MAX_PERF_ENTRIES,
            perf_before_request: PerfBucket::default(),
            perf_headers_received: PerfBucket::default(),
        }
    }
}

thread_local! {
    static RUNTIME_STATE: RefCell<RuntimeState> = RefCell::new(RuntimeState::default());
}

const REMOVEPARAM_TTL_MS: u64 = 10_000;
const MAX_SCRIPTLETS: usize = 32;
const MAX_SCRIPTLET_ARGS: usize = 8;
const MAX_PROCEDURAL_RULES: usize = 64;
const MAX_TRACE_ENTRIES: usize = 50_000;
const MAX_TRACE_ENTRIES_UPPER: usize = 500_000;
const MAX_PERF_ENTRIES: usize = 100_000;
const MAX_PERF_ENTRIES_UPPER: usize = 1_000_000;

fn with_runtime<R>(f: impl FnOnce(&mut RuntimeState) -> R) -> R {
    RUNTIME_STATE.with(|state| {
        let mut state = state.borrow_mut();
        f(&mut state)
    })
}

fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

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
    for selector in result.procedural.into_iter().take(MAX_PROCEDURAL_RULES) {
        if let Some((base, ops)) = parse_procedural_rule(&selector) {
            let ops_array = js_sys::Array::new();
            for op in ops {
                let op_obj = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&op_obj, &"type".into(), &JsValue::from_str(&op.op_type));
                let _ = js_sys::Reflect::set(&op_obj, &"args".into(), &JsValue::from_str(&op.args));
                ops_array.push(&op_obj);
            }
            let rule_obj = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&rule_obj, &"base".into(), &JsValue::from_str(&base));
            let _ = js_sys::Reflect::set(&rule_obj, &"ops".into(), &ops_array);
            procedural.push(&rule_obj);
        }
    }
    let _ = js_sys::Reflect::set(&js_result, &"procedural".into(), &procedural);

    let scriptlets = js_sys::Array::new();
    for call in result.scriptlets.into_iter().take(MAX_SCRIPTLETS) {
        let call_obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&call_obj, &"name".into(), &JsValue::from_str(&call.name));
        let args_array = js_sys::Array::new();
        for arg in call.args.into_iter().take(MAX_SCRIPTLET_ARGS) {
            args_array.push(&parse_scriptlet_arg(&arg));
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

fn get_string_field(value: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(value, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_string())
}

fn normalize_pattern(value: Option<String>) -> String {
    let trimmed = value.unwrap_or_default().trim().to_string();
    if trimmed.is_empty() {
        "*".to_string()
    } else {
        trimmed
    }
}

fn host_matches(pattern: &str, host: &str) -> bool {
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    if host.is_empty() {
        return false;
    }
    if host == pattern {
        return true;
    }
    host.ends_with(&format!(".{pattern}"))
}

fn target_matches(pattern: &str, req_host: &str, req_etld1: &str, is_third_party: bool) -> bool {
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    if pattern == "3p" || pattern == "third-party" {
        return is_third_party;
    }
    if pattern == "1p" || pattern == "first-party" {
        return !is_third_party;
    }
    if !req_etld1.is_empty() && req_etld1 == pattern {
        return true;
    }
    host_matches(pattern, req_host)
}

fn type_matches(rule_type: &str, request_type: &str) -> bool {
    if rule_type.is_empty() || rule_type == "*" {
        return true;
    }
    let normalized = rule_type.to_lowercase();
    match normalized.as_str() {
        "document" => request_type == "main_frame" || request_type == "sub_frame",
        "subdocument" | "sub_frame" => request_type == "sub_frame",
        "main_frame" => request_type == "main_frame",
        "xhr" => request_type == "xmlhttprequest",
        _ => normalized == request_type,
    }
}

fn is_overly_broad_dynamic_rule(rule: &DynamicRule) -> bool {
    let site_pattern = rule.site.to_lowercase();
    let target_pattern = rule.target.to_lowercase();
    let type_pattern = rule.rule_type.to_lowercase();
    let is_global_site = site_pattern == "*";
    let is_global_target = target_pattern == "*";
    let is_main_frame_type = type_pattern == "*" || type_pattern == "main_frame" || type_pattern == "document";
    is_global_site && is_global_target && is_main_frame_type
}

fn parse_dynamic_rules(value: JsValue) -> Result<Vec<DynamicRule>, JsValue> {
    let array = js_sys::Array::from(&value);
    let mut rules = Vec::with_capacity(array.length() as usize);

    for entry in array.iter() {
        let site = normalize_pattern(get_string_field(&entry, "site"));
        let target = normalize_pattern(get_string_field(&entry, "target"));
        let rule_type = normalize_pattern(get_string_field(&entry, "type"));
        let action_val = js_sys::Reflect::get(&entry, &JsValue::from_str("action"))
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as u8;
        let action = DynamicAction::from_u8(action_val);
        rules.push(DynamicRule {
            site,
            target,
            rule_type,
            action,
        });
    }

    Ok(rules)
}

fn parse_string_array(value: JsValue) -> Vec<String> {
    let array = js_sys::Array::from(&value);
    array
        .iter()
        .filter_map(|entry| entry.as_string())
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect()
}

#[wasm_bindgen]
pub fn set_dynamic_rules(value: JsValue) -> Result<(), JsValue> {
    let rules = parse_dynamic_rules(value)?;
    with_runtime(|state| {
        state.dynamic_rules = rules;
    });
    Ok(())
}

#[wasm_bindgen]
pub fn set_runtime_settings(value: JsValue) -> Result<(), JsValue> {
    with_runtime(|state| {
        if let Ok(val) = js_sys::Reflect::get(&value, &JsValue::from_str("dynamicFilteringEnabled")) {
            if let Some(enabled) = val.as_bool() {
                state.settings.dynamic_filtering_enabled = enabled;
            }
        }
        if let Ok(val) = js_sys::Reflect::get(&value, &JsValue::from_str("disabledSites")) {
            if !val.is_undefined() && !val.is_null() {
                state.settings.disabled_sites = parse_string_array(val);
            }
        }
    });
    Ok(())
}

#[wasm_bindgen]
pub fn get_site_pattern_js(url: &str) -> Option<String> {
    let host = extract_host(url)?;
    let etld1 = get_etld1(host);
    if !etld1.is_empty() {
        return Some(etld1);
    }
    Some(host.to_string())
}

#[wasm_bindgen]
pub fn is_site_disabled_js(url: &str) -> bool {
    let host = match extract_host(url) {
        Some(host) => host,
        None => return false,
    };
    with_runtime(|state| state.settings.disabled_sites.iter().any(|pattern| host_matches(pattern, host)))
}

#[wasm_bindgen]
pub fn match_dynamic(url: &str, request_type: &str, initiator: Option<String>) -> JsValue {
    let (action, is_overly_broad) = with_runtime(|state| {
        if !state.settings.dynamic_filtering_enabled || state.dynamic_rules.is_empty() {
            return (DynamicAction::Noop, false);
        }

        let req_host = extract_host(url).unwrap_or("");
        let site_url = initiator.as_deref().unwrap_or(url);
        let site_host = extract_host(site_url).unwrap_or("");
        let site_etld1 = get_etld1(site_host);
        let req_etld1 = get_etld1(req_host);
        let is_third_party = !site_etld1.is_empty() && !req_etld1.is_empty() && site_etld1 != req_etld1;

        let mut best_action = DynamicAction::Noop;
        let mut best_rule: Option<&DynamicRule> = None;
        let mut best_score = -1i32;
        let mut best_index = -1i32;

        for (idx, rule) in state.dynamic_rules.iter().enumerate() {
            let site_pattern = rule.site.to_lowercase();
            let target_pattern = rule.target.to_lowercase();
            let type_pattern = rule.rule_type.to_lowercase();

            if !host_matches(&site_pattern, site_host) {
                continue;
            }
            if !target_matches(&target_pattern, req_host, &req_etld1, is_third_party) {
                continue;
            }
            if !type_matches(&type_pattern, request_type) {
                continue;
            }

            let mut score = 0i32;
            if site_pattern != "*" {
                score += 1;
            }
            if target_pattern != "*" {
                score += 1;
            }
            if type_pattern != "*" {
                score += 1;
            }

            if score > best_score || (score == best_score && idx as i32 > best_index) {
                best_score = score;
                best_index = idx as i32;
                best_action = rule.action;
                best_rule = Some(rule);
            }
        }

        let is_main_frame = request_type == "main_frame" || request_type == "document";
        if best_action == DynamicAction::Block && is_main_frame {
            if let Some(rule) = best_rule {
                if is_overly_broad_dynamic_rule(rule) {
                    return (DynamicAction::Noop, true);
                }
            }
        }

        (best_action, false)
    });

    let result = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("action"), &JsValue::from(action as u8));
    let _ = js_sys::Reflect::set(
        &result,
        &JsValue::from_str("isOverlyBroad"),
        &JsValue::from(is_overly_broad),
    );
    result.into()
}

#[wasm_bindgen]
pub fn removeparam_should_skip(tab_id: i32, frame_id: i32, url: &str, redirect_url: &str) -> bool {
    let key = format!("{tab_id}:{frame_id}:{url}");
    let now = now_ms();
    with_runtime(|state| {
        state
            .removeparam_redirects
            .retain(|_, entry| now.saturating_sub(entry.ts) < REMOVEPARAM_TTL_MS);
        if let Some(entry) = state.removeparam_redirects.get(&key) {
            if now.saturating_sub(entry.ts) < REMOVEPARAM_TTL_MS {
                return true;
            }
        }
        state.removeparam_redirects.insert(
            key,
            RemoveparamEntry {
                ts: now,
                url: redirect_url.to_string(),
            },
        );
        false
    })
}

#[wasm_bindgen]
pub fn removeparam_clear_tab(tab_id: i32) {
    let prefix = format!("{tab_id}:");
    with_runtime(|state| {
        state
            .removeparam_redirects
            .retain(|key, _| !key.starts_with(&prefix));
    });
}

#[wasm_bindgen]
pub fn trace_configure(enabled: bool, max_entries: u32) {
    with_runtime(|state| {
        state.trace_enabled = enabled;
        let max = if max_entries == 0 { MAX_TRACE_ENTRIES as u32 } else { max_entries };
        let clamped = max
            .max(1_000)
            .min(MAX_TRACE_ENTRIES_UPPER as u32) as usize;
        state.trace_max_entries = clamped;
        if !enabled {
            state.trace_entries.clear();
        }
    });
}

#[wasm_bindgen]
pub fn trace_record(
    url: &str,
    request_type: &str,
    initiator: Option<String>,
    tab_id: i32,
    frame_id: i32,
    request_id: &str,
) {
    if url.is_empty() {
        return;
    }
    with_runtime(|state| {
        if !state.trace_enabled {
            return;
        }
        if state.trace_entries.len() >= state.trace_max_entries {
            return;
        }
        state.trace_entries.push(TraceEntry {
            url: url.to_string(),
            request_type: request_type.to_string(),
            initiator,
            tab_id,
            frame_id,
            request_id: request_id.to_string(),
        });
    });
}

#[wasm_bindgen]
pub fn trace_stats() -> JsValue {
    let (enabled, count, max) = with_runtime(|state| {
        (state.trace_enabled, state.trace_entries.len(), state.trace_max_entries)
    });
    let result = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("enabled"), &JsValue::from(enabled));
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("count"), &JsValue::from(count as u32));
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("max"), &JsValue::from(max as u32));
    result.into()
}

#[wasm_bindgen]
pub fn trace_export_jsonl() -> String {
    let entries = with_runtime(|state| state.trace_entries.clone());
    let mut out = String::new();
    for entry in entries {
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("url"), &JsValue::from_str(&entry.url));
        let _ = js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("type"),
            &JsValue::from_str(&entry.request_type),
        );
        if let Some(initiator) = entry.initiator {
            let _ = js_sys::Reflect::set(
                &obj,
                &JsValue::from_str("initiator"),
                &JsValue::from_str(&initiator),
            );
        }
        let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("tabId"), &JsValue::from(entry.tab_id));
        let _ = js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("frameId"),
            &JsValue::from(entry.frame_id),
        );
        let _ = js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("requestId"),
            &JsValue::from_str(&entry.request_id),
        );
        if let Ok(json) = js_sys::JSON::stringify(&obj) {
            if let Some(line) = json.as_string() {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

fn perf_summary(values: &mut Vec<f64>) -> (u32, f64, f64, f64, f64, f64) {
    if values.is_empty() {
        return (0, 0.0, 0.0, 0.0, 0.0, 0.0);
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let count = values.len();
    let min = values[0];
    let max = values[count - 1];
    let p50 = percentile(values, 0.5);
    let p95 = percentile(values, 0.95);
    let p99 = percentile(values, 0.99);
    (count as u32, min, max, p50, p95, p99)
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let idx = (values.len() as f64 * p).floor() as usize;
    let idx = idx.min(values.len() - 1);
    values[idx]
}

#[wasm_bindgen]
pub fn perf_configure(enabled: bool, max_entries: u32) {
    with_runtime(|state| {
        state.perf_enabled = enabled;
        let max = if max_entries == 0 { MAX_PERF_ENTRIES as u32 } else { max_entries };
        let clamped = max
            .max(1_000)
            .min(MAX_PERF_ENTRIES_UPPER as u32) as usize;
        state.perf_max_entries = clamped;
        if !enabled {
            state.perf_before_request.values.clear();
            state.perf_headers_received.values.clear();
        }
    });
}

#[wasm_bindgen]
pub fn perf_record(phase: u8, duration_ms: f64) {
    with_runtime(|state| {
        if !state.perf_enabled {
            return;
        }
        let bucket = match phase {
            0 => &mut state.perf_before_request.values,
            1 => &mut state.perf_headers_received.values,
            _ => return,
        };
        if bucket.len() >= state.perf_max_entries {
            return;
        }
        bucket.push(duration_ms);
    });
}

#[wasm_bindgen]
pub fn perf_stats() -> JsValue {
    let (before, headers, enabled) = with_runtime(|state| {
        (
            state.perf_before_request.values.clone(),
            state.perf_headers_received.values.clone(),
            state.perf_enabled,
        )
    });
    let mut before_vals = before;
    let mut header_vals = headers;
    let (b_count, b_min, b_max, b_p50, b_p95, b_p99) = perf_summary(&mut before_vals);
    let (h_count, h_min, h_max, h_p50, h_p95, h_p99) = perf_summary(&mut header_vals);

    let before_obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&before_obj, &JsValue::from_str("count"), &JsValue::from(b_count));
    let _ = js_sys::Reflect::set(&before_obj, &JsValue::from_str("min"), &JsValue::from(b_min));
    let _ = js_sys::Reflect::set(&before_obj, &JsValue::from_str("max"), &JsValue::from(b_max));
    let _ = js_sys::Reflect::set(&before_obj, &JsValue::from_str("p50"), &JsValue::from(b_p50));
    let _ = js_sys::Reflect::set(&before_obj, &JsValue::from_str("p95"), &JsValue::from(b_p95));
    let _ = js_sys::Reflect::set(&before_obj, &JsValue::from_str("p99"), &JsValue::from(b_p99));

    let headers_obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&headers_obj, &JsValue::from_str("count"), &JsValue::from(h_count));
    let _ = js_sys::Reflect::set(&headers_obj, &JsValue::from_str("min"), &JsValue::from(h_min));
    let _ = js_sys::Reflect::set(&headers_obj, &JsValue::from_str("max"), &JsValue::from(h_max));
    let _ = js_sys::Reflect::set(&headers_obj, &JsValue::from_str("p50"), &JsValue::from(h_p50));
    let _ = js_sys::Reflect::set(&headers_obj, &JsValue::from_str("p95"), &JsValue::from(h_p95));
    let _ = js_sys::Reflect::set(&headers_obj, &JsValue::from_str("p99"), &JsValue::from(h_p99));

    let result = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("enabled"), &JsValue::from(enabled));
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("beforeRequest"), &before_obj);
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("headersReceived"), &headers_obj);
    result.into()
}

#[wasm_bindgen]
pub fn perf_export_json() -> String {
    let (before, headers) = with_runtime(|state| {
        (
            state.perf_before_request.values.clone(),
            state.perf_headers_received.values.clone(),
        )
    });
    let result = js_sys::Object::new();
    let before_array = js_sys::Array::new();
    for value in before {
        before_array.push(&JsValue::from(value));
    }
    let headers_array = js_sys::Array::new();
    for value in headers {
        headers_array.push(&JsValue::from(value));
    }
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("beforeRequest"), &before_array);
    let _ = js_sys::Reflect::set(&result, &JsValue::from_str("headersReceived"), &headers_array);
    js_sys::JSON::stringify(&result)
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default()
}

fn is_numeric_literal(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let mut chars = value.chars().peekable();
    if matches!(chars.peek(), Some('-')) {
        chars.next();
    }
    let mut int_digits = 0usize;
    while let Some(ch) = chars.peek() {
        if ch.is_ascii_digit() {
            int_digits += 1;
            chars.next();
        } else {
            break;
        }
    }
    if int_digits == 0 {
        return false;
    }
    if int_digits > 1 && value.trim_start_matches('-').starts_with('0') {
        return false;
    }
    let mut frac_digits = 0usize;
    if matches!(chars.peek(), Some('.')) {
        chars.next();
        while let Some(ch) = chars.peek() {
            if ch.is_ascii_digit() {
                frac_digits += 1;
                chars.next();
            } else {
                break;
            }
        }
        if frac_digits == 0 {
            return false;
        }
        if let Some(last) = value.chars().last() {
            if last == '0' {
                return false;
            }
        }
    }
    chars.next().is_none()
}

fn parse_scriptlet_arg(raw: &str) -> JsValue {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return JsValue::NULL;
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return JsValue::from(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return JsValue::from(false);
    }
    if trimmed.eq_ignore_ascii_case("undefined") {
        return JsValue::UNDEFINED;
    }
    if trimmed.is_empty() {
        return JsValue::from_str("");
    }
    if is_numeric_literal(trimmed) {
        if let Ok(value) = trimmed.parse::<f64>() {
            return JsValue::from(value);
        }
    }
    JsValue::from_str(raw)
}

struct ProceduralOp {
    op_type: String,
    args: String,
}

struct ProceduralToken {
    op_type: &'static str,
    token: &'static str,
}

const PROCEDURAL_TOKENS: [ProceduralToken; 6] = [
    ProceduralToken {
        op_type: "has-text",
        token: ":has-text(",
    },
    ProceduralToken {
        op_type: "matches-css",
        token: ":matches-css(",
    },
    ProceduralToken {
        op_type: "xpath",
        token: ":xpath(",
    },
    ProceduralToken {
        op_type: "upward",
        token: ":upward(",
    },
    ProceduralToken {
        op_type: "remove",
        token: ":remove(",
    },
    ProceduralToken {
        op_type: "style",
        token: ":style(",
    },
];

fn find_next_procedural_op(raw: &str, start: usize) -> Option<(usize, &'static ProceduralToken)> {
    let mut best: Option<(usize, &'static ProceduralToken)> = None;
    for token in PROCEDURAL_TOKENS.iter() {
        if let Some(idx) = raw[start..].find(token.token) {
            let index = start + idx;
            if best.map_or(true, |(best_idx, _)| index < best_idx) {
                best = Some((index, token));
            }
        }
    }
    best
}

fn read_paren_content(raw: &str, start: usize) -> Option<(String, usize)> {
    let bytes = raw.as_bytes();
    if bytes.get(start) != Some(&b'(') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((raw[start + 1..i].to_string(), i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_procedural_rule(raw: &str) -> Option<(String, Vec<ProceduralOp>)> {
    let first = find_next_procedural_op(raw, 0)?;
    let base = raw[..first.0].trim();
    let mut ops = Vec::new();
    let mut cursor = first.0;
    while cursor < raw.len() {
        let next = find_next_procedural_op(raw, cursor);
        let Some((index, token)) = next else { break };
        let paren_start = index + token.token.len() - 1;
        let parsed = read_paren_content(raw, paren_start)?;
        ops.push(ProceduralOp {
            op_type: token.op_type.to_string(),
            args: parsed.0.trim().to_string(),
        });
        cursor = parsed.1 + 1;
    }
    if ops.is_empty() {
        return None;
    }
    let base_selector = if base.is_empty() { "*" } else { base };
    Some((base_selector.to_string(), ops))
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
