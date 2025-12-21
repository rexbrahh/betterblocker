/**
 * Core type definitions for BetterBlocker
 * 
 * These types map directly to the UBX snapshot binary format and
 * are used throughout the matching engine.
 */

// =============================================================================
// Rule Actions (matches RULES section action field)
// =============================================================================

export const enum RuleAction {
  /** Exception rule (@@...) - allows the request */
  ALLOW = 0,
  /** Block rule - cancels the request */
  BLOCK = 1,
  /** Redirect directive (redirect-rule) - redirects blocked requests */
  REDIRECT_DIRECTIVE = 2,
  /** Remove URL parameters */
  REMOVEPARAM = 3,
  /** Inject CSP header */
  CSP_INJECT = 4,
  /** Block based on response header match */
  HEADER_MATCH_BLOCK = 5,
  /** Allow based on response header match (exception) */
  HEADER_MATCH_ALLOW = 6,
  /** Cancel at response phase (rare) */
  RESPONSE_CANCEL = 7,
}

// =============================================================================
// Rule Flags (bit flags for RULES section flags field)
// =============================================================================

export const enum RuleFlags {
  /** $important - ignores exception filters */
  IMPORTANT = 1 << 0,
  /** Pattern is a regex */
  IS_REGEX = 1 << 1,
  /** Case-sensitive matching ($match-case) */
  MATCH_CASE = 1 << 2,
  /** Created by $redirect= (block part) */
  FROM_REDIRECT_EQ = 1 << 4,
  /** Created by $redirect= (directive part) */
  REDIRECT_DIRECTIVE_CREATED = 1 << 5,
  /** User-added rule (not from subscription) */
  IS_USER_RULE = 1 << 6,
  /** Rule has right anchor (ends with |) */
  HAS_RIGHT_ANCHOR = 1 << 7,
  /** Rule has hostname anchor (||) */
  HAS_HOST_ANCHOR = 1 << 8,
  /** Rule has left anchor (starts with |) */
  HAS_LEFT_ANCHOR = 1 << 9,
  CSP_EXCEPTION = 1 << 10,
  REDIRECT_RULE_EXCEPTION = 1 << 11,
  ELEMHIDE = 1 << 12,
  GENERICHIDE = 1 << 13,
}

// =============================================================================
// Request Types (bit mask for type filtering)
// =============================================================================

export const enum RequestType {
  OTHER = 1 << 0,
  SCRIPT = 1 << 1,
  IMAGE = 1 << 2,
  STYLESHEET = 1 << 3,
  OBJECT = 1 << 4,
  SUBDOCUMENT = 1 << 5,  // iframe/frame
  MAIN_FRAME = 1 << 6,   // main document
  XMLHTTPREQUEST = 1 << 7,
  WEBSOCKET = 1 << 8,
  FONT = 1 << 9,
  MEDIA = 1 << 10,
  PING = 1 << 11,
  CSP_REPORT = 1 << 12,
  BEACON = 1 << 13,
  FETCH = 1 << 14,
  SPECULATIVE = 1 << 15,
}

/** All request types mask */
export const ALL_REQUEST_TYPES = 0xFFFF;

/** Document types (main_frame + sub_frame) for CSP injection */
export const DOCUMENT_TYPES = RequestType.MAIN_FRAME | RequestType.SUBDOCUMENT;

// =============================================================================
// Party Masks
// =============================================================================

export const enum PartyMask {
  /** Matches first-party requests */
  FIRST_PARTY = 1 << 0,
  /** Matches third-party requests */
  THIRD_PARTY = 1 << 1,
}

/** Matches both first and third party */
export const ALL_PARTIES = PartyMask.FIRST_PARTY | PartyMask.THIRD_PARTY;

// =============================================================================
// Scheme Masks
// =============================================================================

export const enum SchemeMask {
  HTTP = 1 << 0,
  HTTPS = 1 << 1,
  WS = 1 << 2,
  WSS = 1 << 3,
  DATA = 1 << 4,
  FTP = 1 << 5,
}

/** All web schemes */
export const ALL_SCHEMES = 0xFF;

// =============================================================================
// Pattern Bytecode Opcodes
// =============================================================================

export const enum PatternOp {
  /** Find literal substring: FIND_LIT <strOff:u32> <strLen:u16> */
  FIND_LIT = 0x01,
  /** Assert current position is start of URL */
  ASSERT_START = 0x02,
  /** Assert current position is end of URL */
  ASSERT_END = 0x03,
  /** Assert next char is boundary (ABP ^ separator) */
  ASSERT_BOUNDARY = 0x04,
  /** Skip any chars (for * wildcard) */
  SKIP_ANY = 0x05,
  /** Hostname anchor - verify host hash matches */
  HOST_ANCHOR = 0x06,
  /** Pattern match complete */
  DONE = 0x07,
}

// =============================================================================
// Match Result
// =============================================================================

export const enum MatchDecision {
  /** Request is allowed (no matching block rules, or exception matched) */
  ALLOW = 0,
  /** Request is blocked */
  BLOCK = 1,
  /** Request is redirected to a surrogate */
  REDIRECT = 2,
  /** URL parameters were removed (redirect to modified URL) */
  REMOVEPARAM = 3,
}

export interface MatchResult {
  /** The final decision for this request */
  decision: MatchDecision;
  /** Rule ID that determined the decision (for logging) */
  ruleId: number;
  /** List ID the rule came from (for logging) */
  listId: number;
  /** Redirect URL if decision is REDIRECT or REMOVEPARAM */
  redirectUrl?: string;
}

// =============================================================================
// Request Context
// =============================================================================

export interface RequestContext {
  /** Full request URL */
  url: string;
  /** Request hostname (extracted from URL) */
  reqHost: string;
  /** Request eTLD+1 */
  reqETLD1: string;
  /** Context/initiator hostname */
  siteHost: string;
  /** Context/initiator eTLD+1 */
  siteETLD1: string;
  /** Is this a third-party request? */
  isThirdParty: boolean;
  /** Request type as bit flag */
  type: RequestType;
  /** URL scheme as bit flag (0 if unknown) */
  scheme: SchemeMask | 0;
  /** Tab ID */
  tabId: number;
  /** Frame ID */
  frameId: number;
  /** Request ID (for logging) */
  requestId: string;
}

// =============================================================================
// Hash64 type (two 32-bit parts for domain hashing)
// =============================================================================

export interface Hash64 {
  lo: number;
  hi: number;
}

// =============================================================================
// WebRequest types (subset for our use)
// =============================================================================

export type WebRequestType =
  | 'main_frame'
  | 'sub_frame'
  | 'stylesheet'
  | 'script'
  | 'image'
  | 'font'
  | 'object'
  | 'xmlhttprequest'
  | 'ping'
  | 'csp_report'
  | 'media'
  | 'websocket'
  | 'other';

/** Map browser request type string to our bit flag */
export function requestTypeFromString(type: WebRequestType): RequestType {
  switch (type) {
    case 'main_frame':
      return RequestType.MAIN_FRAME;
    case 'sub_frame':
      return RequestType.SUBDOCUMENT;
    case 'stylesheet':
      return RequestType.STYLESHEET;
    case 'script':
      return RequestType.SCRIPT;
    case 'image':
      return RequestType.IMAGE;
    case 'font':
      return RequestType.FONT;
    case 'object':
      return RequestType.OBJECT;
    case 'xmlhttprequest':
      return RequestType.XMLHTTPREQUEST;
    case 'ping':
      return RequestType.PING;
    case 'csp_report':
      return RequestType.CSP_REPORT;
    case 'media':
      return RequestType.MEDIA;
    case 'websocket':
      return RequestType.WEBSOCKET;
    default:
      return RequestType.OTHER;
  }
}

// =============================================================================
// Cosmetic filter types
// =============================================================================

export const enum CosmeticAction {
  /** Hide element with CSS */
  HIDE = 0,
  /** Exception - don't hide */
  UNHIDE = 1,
}

export interface CosmeticPayload {
  /** CSS text to inject (combined selectors) */
  css: string;
  /** Whether generic cosmetics are enabled for this page */
  enableGeneric: boolean;
  /** Procedural filter programs to execute */
  procedural: ProceduralProgram[];
  /** Scriptlets to inject */
  scriptlets: ScriptletCall[];
}

export interface ProceduralProgram {
  /** Operator chain (e.g., has-text, xpath, etc.) */
  operators: ProceduralOperator[];
}

export interface ProceduralOperator {
  /** Operator type */
  type: string;
  /** Operator arguments */
  args: string;
}

export interface ScriptletCall {
  /** Scriptlet function name */
  name: string;
  /** Arguments to pass */
  args: string[];
}

// =============================================================================
// Logger entry
// =============================================================================

export interface LogEntry {
  /** Timestamp (performance.now()) */
  timestamp: number;
  /** Tab ID */
  tabId: number;
  /** Request ID */
  requestId: string;
  /** Truncated URL (for display) */
  url: string;
  /** Request type */
  type: RequestType;
  /** First or third party */
  isThirdParty: boolean;
  /** Decision made */
  decision: MatchDecision;
  /** Rule ID that matched */
  ruleId: number;
  /** List ID */
  listId: number;
}

// =============================================================================
// Dynamic filtering types
// =============================================================================

export const enum DynamicAction {
  /** No override, use static filters */
  NOOP = 0,
  /** Block */
  BLOCK = 1,
  /** Allow */
  ALLOW = 2,
}

export interface DynamicRule {
  /** Site pattern (*, eTLD+1, or full host) */
  site: string;
  /** Target pattern (*, 3p, or specific eTLD+1) */
  target: string;
  /** Request type (or * for all) */
  type: string;
  /** Action */
  action: DynamicAction;
}

export interface UserSettings {
  enabled: boolean;
  cosmeticsEnabled: boolean;
  scriptletsEnabled: boolean;
  dynamicFilteringEnabled: boolean;
  removeparamEnabled: boolean;
  cspEnabled: boolean;
  responseHeaderEnabled: boolean;
  disabledSites: string[];
}

export const DEFAULT_SETTINGS: UserSettings = {
  enabled: true,
  cosmeticsEnabled: true,
  scriptletsEnabled: true,
  dynamicFilteringEnabled: true,
  removeparamEnabled: true,
  cspEnabled: true,
  responseHeaderEnabled: true,
  disabledSites: [],
};

// =============================================================================
// WebRequest details interface (browser-agnostic subset)
// =============================================================================

export interface WebRequestDetails {
  requestId: string;
  url: string;
  method: string;
  frameId: number;
  parentFrameId: number;
  tabId: number;
  type: WebRequestType;
  timeStamp: number;
  /** Chromium: initiator origin */
  initiator?: string;
  /** Firefox: document URL */
  documentUrl?: string;
  /** Firefox: origin URL */
  originUrl?: string;
}

// =============================================================================
// Browser compatibility
// =============================================================================

export interface BrowserCompat {
  /** Whether this is Firefox */
  isFirefox: boolean;
  /** Whether this is Chromium-based */
  isChromium: boolean;
  /** Get initiator/origin URL from request details */
  getInitiator(details: WebRequestDetails): string | undefined;
}
