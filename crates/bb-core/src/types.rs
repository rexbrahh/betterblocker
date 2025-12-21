//! Core type definitions for BetterBlocker
//!
//! These types map directly to the UBX snapshot binary format and
//! are used throughout the matching engine.

#[cfg(not(feature = "std"))]
use alloc::string::String;

// =============================================================================
// Rule Actions (matches RULES section action field)
// =============================================================================

/// Action to take for a matched rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RuleAction {
    /// Exception rule (@@...) - allows the request
    Allow = 0,
    /// Block rule - cancels the request
    Block = 1,
    /// Redirect directive (redirect-rule) - redirects blocked requests
    RedirectDirective = 2,
    /// Remove URL parameters
    Removeparam = 3,
    /// Inject CSP header
    CspInject = 4,
    /// Block based on response header match
    HeaderMatchBlock = 5,
    /// Allow based on response header match (exception)
    HeaderMatchAllow = 6,
    /// Cancel at response phase (rare)
    ResponseCancel = 7,
}

impl TryFrom<u8> for RuleAction {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Allow),
            1 => Ok(Self::Block),
            2 => Ok(Self::RedirectDirective),
            3 => Ok(Self::Removeparam),
            4 => Ok(Self::CspInject),
            5 => Ok(Self::HeaderMatchBlock),
            6 => Ok(Self::HeaderMatchAllow),
            7 => Ok(Self::ResponseCancel),
            _ => Err(()),
        }
    }
}

// =============================================================================
// Rule Flags (bit flags for RULES section flags field)
// =============================================================================

bitflags::bitflags! {
    /// Flags for rule behavior.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct RuleFlags: u16 {
        /// $important - ignores exception filters
        const IMPORTANT = 1 << 0;
        /// Pattern is a regex
        const IS_REGEX = 1 << 1;
        /// Case-sensitive matching ($match-case)
        const MATCH_CASE = 1 << 2;
        /// Created by $redirect= (block part)
        const FROM_REDIRECT_EQ = 1 << 4;
        /// Created by $redirect= (directive part)
        const REDIRECT_DIRECTIVE_CREATED = 1 << 5;
        /// User-added rule (not from subscription)
        const IS_USER_RULE = 1 << 6;
        /// Rule has right anchor (ends with |)
        const HAS_RIGHT_ANCHOR = 1 << 7;
        /// Rule has hostname anchor (||)
        const HAS_HOST_ANCHOR = 1 << 8;
        /// Rule has left anchor (starts with |)
        const HAS_LEFT_ANCHOR = 1 << 9;
    }
}

// =============================================================================
// Request Types (bit mask for type filtering)
// =============================================================================

bitflags::bitflags! {
    /// Request type bit mask.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct RequestType: u32 {
        const OTHER = 1 << 0;
        const SCRIPT = 1 << 1;
        const IMAGE = 1 << 2;
        const STYLESHEET = 1 << 3;
        const OBJECT = 1 << 4;
        const SUBDOCUMENT = 1 << 5;  // iframe/frame
        const MAIN_FRAME = 1 << 6;   // main document
        const XMLHTTPREQUEST = 1 << 7;
        const WEBSOCKET = 1 << 8;
        const FONT = 1 << 9;
        const MEDIA = 1 << 10;
        const PING = 1 << 11;
        const CSP_REPORT = 1 << 12;
        const BEACON = 1 << 13;
        const FETCH = 1 << 14;
        const SPECULATIVE = 1 << 15;
        
        /// All request types
        const ALL = 0xFFFF;
        /// Document types (main_frame + sub_frame)
        const DOCUMENT = Self::MAIN_FRAME.bits() | Self::SUBDOCUMENT.bits();
    }
}

impl RequestType {
    /// Parse from browser request type string.
    pub fn from_str(s: &str) -> Self {
        match s {
            "main_frame" => Self::MAIN_FRAME,
            "sub_frame" => Self::SUBDOCUMENT,
            "stylesheet" => Self::STYLESHEET,
            "script" => Self::SCRIPT,
            "image" => Self::IMAGE,
            "font" => Self::FONT,
            "object" => Self::OBJECT,
            "xmlhttprequest" => Self::XMLHTTPREQUEST,
            "ping" => Self::PING,
            "csp_report" => Self::CSP_REPORT,
            "media" => Self::MEDIA,
            "websocket" => Self::WEBSOCKET,
            _ => Self::OTHER,
        }
    }
}

// =============================================================================
// Party Masks
// =============================================================================

bitflags::bitflags! {
    /// Party (first-party / third-party) mask.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PartyMask: u8 {
        /// Matches first-party requests
        const FIRST_PARTY = 1 << 0;
        /// Matches third-party requests
        const THIRD_PARTY = 1 << 1;
        /// Matches both
        const ALL = Self::FIRST_PARTY.bits() | Self::THIRD_PARTY.bits();
    }
}

// =============================================================================
// Scheme Masks
// =============================================================================

bitflags::bitflags! {
    /// URL scheme mask.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct SchemeMask: u8 {
        const HTTP = 1 << 0;
        const HTTPS = 1 << 1;
        const WS = 1 << 2;
        const WSS = 1 << 3;
        const DATA = 1 << 4;
        const FTP = 1 << 5;
        /// All web schemes
        const ALL = 0xFF;
    }
}

// =============================================================================
// Pattern Bytecode Opcodes
// =============================================================================

/// Pattern bytecode opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PatternOp {
    /// Find literal substring: FIND_LIT <strOff:u32> <strLen:u16>
    FindLit = 0x01,
    /// Assert current position is start of URL
    AssertStart = 0x02,
    /// Assert current position is end of URL
    AssertEnd = 0x03,
    /// Assert next char is boundary (ABP ^ separator)
    AssertBoundary = 0x04,
    /// Skip any chars (for * wildcard)
    SkipAny = 0x05,
    /// Hostname anchor - verify host hash matches
    HostAnchor = 0x06,
    /// Pattern match complete
    Done = 0x07,
}

impl TryFrom<u8> for PatternOp {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::FindLit),
            0x02 => Ok(Self::AssertStart),
            0x03 => Ok(Self::AssertEnd),
            0x04 => Ok(Self::AssertBoundary),
            0x05 => Ok(Self::SkipAny),
            0x06 => Ok(Self::HostAnchor),
            0x07 => Ok(Self::Done),
            _ => Err(()),
        }
    }
}

// =============================================================================
// Request Context
// =============================================================================

/// Context for a request being matched.
#[derive(Debug, Clone)]
pub struct RequestContext<'a> {
    /// Full request URL
    pub url: &'a str,
    /// Request hostname (extracted from URL)
    pub req_host: &'a str,
    /// Request eTLD+1
    pub req_etld1: &'a str,
    /// Context/initiator hostname
    pub site_host: &'a str,
    /// Context/initiator eTLD+1
    pub site_etld1: &'a str,
    /// Is this a third-party request?
    pub is_third_party: bool,
    /// Request type
    pub request_type: RequestType,
    /// URL scheme
    pub scheme: SchemeMask,
    /// Tab ID
    pub tab_id: i32,
    /// Frame ID
    pub frame_id: i32,
    /// Request ID (for logging)
    pub request_id: &'a str,
}

// =============================================================================
// Match Result
// =============================================================================

/// Final decision for a matched request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchDecision {
    /// Request is allowed (no matching block rules, or exception matched)
    Allow,
    /// Request is blocked
    Block,
    /// Request is redirected to a surrogate
    Redirect,
    /// URL parameters were removed (redirect to modified URL)
    Removeparam,
}

/// Result of matching a request.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// The final decision for this request
    pub decision: MatchDecision,
    /// Rule ID that determined the decision (for logging)
    pub rule_id: i32,
    /// List ID the rule came from (for logging)
    pub list_id: u16,
    /// Redirect URL if decision is Redirect or Removeparam
    pub redirect_url: Option<String>,
}

impl Default for MatchResult {
    fn default() -> Self {
        Self {
            decision: MatchDecision::Allow,
            rule_id: -1,
            list_id: 0,
            redirect_url: None,
        }
    }
}
