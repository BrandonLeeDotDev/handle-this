//! Core error type and related structures.

#[cfg(not(feature = "std"))]
use alloc::{
    borrow::Cow,
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(feature = "std")]
use std::borrow::Cow;
#[cfg(feature = "std")]
use std::sync::OnceLock;

use core::fmt;

#[cfg(feature = "std")]
use std::error::Error as StdError;

// ============================================================
// Core types
// ============================================================

/// Error wrapper that captures context and stack traces for any error.
///
/// `Handled<E>` wraps an error type `E` while adding:
/// - A trace of frames showing where the error propagated
/// - Context messages and key-value attachments at each frame
///
/// # Type Parameters
///
/// - `E` - The underlying error type. Defaults to `Error` for type-erased
///   errors (the common case with `handle!` macro).
///
/// The `handle!` macro produces `Handled<Error>` (aliased as `Handled`),
/// with the concrete error type preserved inside for downcasting via `TryCatch`.
///
/// # Examples
///
/// ```
/// use handle_this::{handle, Handled, Result};
///
/// fn read_file(path: &str) -> Result<String> {
///     handle! { try { std::fs::read_to_string(path)? } with "reading config" }
/// }
/// ```
#[derive(Debug)]
pub struct Handled<E = Error> {
    pub(crate) source: E,
    /// Lazy message - only computed when accessed via `message()`.
    /// This avoids expensive `to_string()` calls on every error creation.
    pub(crate) message: OnceLock<String>,
    /// Location trace - inline storage for ≤8 frames (common case), heap for overflow.
    /// Avoids allocation for typical error traces.
    pub(crate) locations: LocationVec,
    /// Context entries - expensive, only allocated when .ctx()/.kv() used.
    /// Each entry references a location by index.
    pub(crate) contexts: Option<Vec<ContextEntry>>,
    /// Previous error in chain - used by `chain_after` for `catch any/all`.
    /// Stored separately to preserve the root error's type for `catch Type`.
    #[cfg(feature = "std")]
    pub(crate) chained: Option<Box<Handled<Error>>>,
}

/// Type-erased error wrapper for when you don't need to preserve the concrete type.
///
/// This is a newtype wrapper around `Box<dyn StdError>` that enables proper trait
/// resolution for typed catches. It does NOT implement `Error` itself (to avoid
/// trait impl conflicts), but provides access to the inner error.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct Error(Box<dyn StdError + Send + Sync + 'static>);

#[cfg(not(feature = "std"))]
#[derive(Debug)]
pub struct Error(Box<dyn fmt::Debug + fmt::Display + Send + Sync + 'static>);

#[cfg(feature = "std")]
impl Error {
    /// Create from any error type.
    #[inline]
    pub fn new<E: StdError + Send + Sync + 'static>(e: E) -> Self {
        Self(Box::new(e))
    }

    /// Create from a boxed error.
    #[inline]
    pub fn from_box(e: Box<dyn StdError + Send + Sync + 'static>) -> Self {
        Self(e)
    }

    /// Get the inner error as a trait object reference.
    #[inline]
    pub fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
        self.0.as_ref()
    }

    /// Get the inner error as a trait object reference (non-Send/Sync for Error trait compat).
    #[inline]
    pub fn as_dyn_error(&self) -> &(dyn StdError + 'static) {
        self.0.as_ref()
    }

    /// Try to downcast to a specific error type.
    #[inline]
    pub fn downcast_ref<T: StdError + 'static>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }

    /// Try to downcast and consume the error.
    #[inline]
    pub fn downcast<T: StdError + 'static>(self) -> core::result::Result<T, Self> {
        match self.0.downcast::<T>() {
            Ok(e) => Ok(*e),
            Err(e) => Err(Self(e)),
        }
    }

    /// Get the inner boxed error.
    pub fn into_inner(self) -> Box<dyn StdError + Send + Sync + 'static> {
        self.0
    }
}

#[cfg(not(feature = "std"))]
impl Error {
    /// Create from any error type.
    #[inline]
    pub fn new<E: fmt::Debug + fmt::Display + Send + Sync + 'static>(e: E) -> Self {
        Self(Box::new(e))
    }

    /// Create from a boxed error.
    #[inline]
    pub fn from_box(e: Box<dyn fmt::Debug + fmt::Display + Send + Sync + 'static>) -> Self {
        Self(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// From impl for Error - enables ? operator in try blocks
// This doesn't conflict with From<T> for T because Error doesn't implement Error
#[cfg(feature = "std")]
impl<E: StdError + Send + Sync + 'static> From<E> for Error {
    fn from(e: E) -> Self {
        Error::new(e)
    }
}

/// Location in source code - cheap, no allocation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Location {
    pub(crate) file: &'static str,  // Always from file!() macro
    pub(crate) line: u32,
    pub(crate) col: u32,
}

/// Inline storage for locations - avoids heap allocation for common case.
/// Stores up to 4 frames inline (covers most error traces); overflows to Vec for deeper traces.
const INLINE_CAPACITY: usize = 4;

#[derive(Debug)]
pub(crate) struct LocationVec {
    len: u8,
    inline: [core::mem::MaybeUninit<Location>; INLINE_CAPACITY],
    overflow: Option<Vec<Location>>,
}

impl Clone for LocationVec {
    fn clone(&self) -> Self {
        let mut new = Self::new();
        for loc in self.iter() {
            new.push(*loc);
        }
        new
    }
}

impl LocationVec {
    #[inline]
    pub const fn new() -> Self {
        Self {
            len: 0,
            inline: [core::mem::MaybeUninit::uninit(); INLINE_CAPACITY],
            overflow: None,
        }
    }

    #[inline]
    pub fn push(&mut self, loc: Location) {
        let idx = self.len as usize;
        if idx < INLINE_CAPACITY {
            self.inline[idx] = core::mem::MaybeUninit::new(loc);
            self.len += 1;
        } else if idx < DEFAULT_LOCATION_LIMIT {
            // Spill to overflow
            let overflow = self.overflow.get_or_insert_with(Vec::new);
            overflow.push(loc);
            self.len += 1;
        }
        // Silently drop if at limit
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &Location> + '_ {
        let inline_count = core::cmp::min(self.len as usize, INLINE_CAPACITY);
        let inline_iter = (0..inline_count).map(move |i| {
            // SAFETY: We only read initialized elements (i < inline_count)
            unsafe { self.inline[i].assume_init_ref() }
        });
        let overflow_iter = self.overflow.iter().flat_map(|v| v.iter());
        inline_iter.chain(overflow_iter)
    }
}

/// Context entry attached to a specific location - expensive, has allocations.
#[derive(Debug, Clone)]
pub(crate) struct ContextEntry {
    pub(crate) location_idx: u16,  // Which location this attaches to
    pub(crate) message: Option<String>,
    pub(crate) attachments: Vec<(Cow<'static, str>, Value)>,
}

/// A typed value for structured logging attachments.
///
/// Preserves type information for JSON serialization and log aggregation systems.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// String value
    String(String),
    /// Signed integer (i8, i16, i32, i64, isize)
    Int(i64),
    /// Unsigned integer (u8, u16, u32, u64, usize)
    Uint(u64),
    /// Floating point (f32, f64)
    Float(f64),
    /// Boolean
    Bool(bool),
    /// Null/None value
    Null,
}

impl Value {
    /// Create a Value from any supported type.
    pub fn from<T: IntoValue>(v: T) -> Self {
        v.into_value()
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::String(s) => write!(f, "{}", s),
            Value::Int(n) => write!(f, "{}", n),
            Value::Uint(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Null => write!(f, "null"),
        }
    }
}

// Allow comparing Value with string types for convenience in tests
impl PartialEq<str> for Value {
    fn eq(&self, other: &str) -> bool {
        match self {
            Value::String(s) => s == other,
            _ => false,
        }
    }
}

impl PartialEq<&str> for Value {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

impl PartialEq<String> for Value {
    fn eq(&self, other: &String) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<i64> for Value {
    fn eq(&self, other: &i64) -> bool {
        match self {
            Value::Int(n) => n == other,
            _ => false,
        }
    }
}

impl PartialEq<u64> for Value {
    fn eq(&self, other: &u64) -> bool {
        match self {
            Value::Uint(n) => n == other,
            _ => false,
        }
    }
}

impl PartialEq<f64> for Value {
    fn eq(&self, other: &f64) -> bool {
        match self {
            Value::Float(n) => (n - other).abs() < f64::EPSILON,
            _ => false,
        }
    }
}

impl PartialEq<bool> for Value {
    fn eq(&self, other: &bool) -> bool {
        match self {
            Value::Bool(b) => b == other,
            _ => false,
        }
    }
}

/// Trait for converting types into Value.
pub trait IntoValue {
    fn into_value(self) -> Value;
}

impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}

impl IntoValue for String {
    fn into_value(self) -> Value {
        Value::String(self)
    }
}

impl IntoValue for &str {
    fn into_value(self) -> Value {
        Value::String(self.to_string())
    }
}

impl<'a> IntoValue for Cow<'a, str> {
    fn into_value(self) -> Value {
        Value::String(self.into_owned())
    }
}

impl IntoValue for bool {
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
}

impl IntoValue for i8 {
    fn into_value(self) -> Value {
        Value::Int(self as i64)
    }
}

impl IntoValue for i16 {
    fn into_value(self) -> Value {
        Value::Int(self as i64)
    }
}

impl IntoValue for i32 {
    fn into_value(self) -> Value {
        Value::Int(self as i64)
    }
}

impl IntoValue for i64 {
    fn into_value(self) -> Value {
        Value::Int(self)
    }
}

impl IntoValue for isize {
    fn into_value(self) -> Value {
        Value::Int(self as i64)
    }
}

impl IntoValue for u8 {
    fn into_value(self) -> Value {
        Value::Uint(self as u64)
    }
}

impl IntoValue for u16 {
    fn into_value(self) -> Value {
        Value::Uint(self as u64)
    }
}

impl IntoValue for u32 {
    fn into_value(self) -> Value {
        Value::Uint(self as u64)
    }
}

impl IntoValue for u64 {
    fn into_value(self) -> Value {
        Value::Uint(self)
    }
}

impl IntoValue for usize {
    fn into_value(self) -> Value {
        Value::Uint(self as u64)
    }
}

impl IntoValue for f32 {
    fn into_value(self) -> Value {
        Value::Float(self as f64)
    }
}

impl IntoValue for f64 {
    fn into_value(self) -> Value {
        Value::Float(self)
    }
}

impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Value {
        match self {
            Some(v) => v.into_value(),
            None => Value::Null,
        }
    }
}

// Reference implementation - deref and convert
impl<T: IntoValue + Clone> IntoValue for &T {
    fn into_value(self) -> Value {
        self.clone().into_value()
    }
}

/// Default limits for trace depth
pub const DEFAULT_LOCATION_LIMIT: usize = 32;
pub const DEFAULT_CONTEXT_LIMIT: usize = 8;

/// View into a single frame of the error trace.
#[derive(Debug, Clone)]
pub struct FrameView<'a> {
    /// Source file path
    pub file: &'a str,
    /// Line number
    pub line: u32,
    /// Column number
    pub col: u32,
    /// Optional context message
    pub context: Option<&'a str>,
    /// Key-value attachments (internal)
    attachments_inner: &'a [(Cow<'static, str>, Value)],
}

impl<'a> FrameView<'a> {
    /// Iterate over key-value attachments on this frame with typed values.
    pub fn attachments(&self) -> impl Iterator<Item = (&'a str, &'a Value)> {
        self.attachments_inner.iter().map(|(k, v)| (k.as_ref(), v))
    }

    /// Iterate over key-value attachments as strings (for backwards compatibility).
    pub fn attachments_str(&self) -> impl Iterator<Item = (&'a str, String)> + 'a {
        self.attachments_inner.iter().map(|(k, v)| (k.as_ref(), v.to_string()))
    }
}

// ============================================================
// TryCatch trait - enables typed catches with both concrete and erased errors
// ============================================================

/// Trait for attempting to extract a specific error type from a Handled wrapper.
///
/// This enables typed `catch Type(e) { }` patterns to work with both:
/// - `Handled<E>` where `E` is the exact type (direct access)
/// - `Handled<Error>` where we need runtime downcasting
#[doc(hidden)]
pub trait TryCatch<Target> {
    fn try_catch(&self) -> Option<&Target>;
}

// Direct match - when the source IS the target type
impl<T> TryCatch<T> for T {
    #[inline]
    fn try_catch(&self) -> Option<&T> {
        Some(self)
    }
}

// Downcast match - for type-erased errors (newtype, doesn't impl Error)
#[cfg(feature = "std")]
impl<T: StdError + 'static> TryCatch<T> for Error {
    #[inline]
    fn try_catch(&self) -> Option<&T> {
        self.downcast_ref::<T>()
    }
}

// ============================================================
// StringError helper
// ============================================================

#[derive(Debug)]
pub struct StringError(pub(crate) String);

impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(feature = "std")]
impl StdError for StringError {}

// ============================================================
// Handled<E> implementation - generic over error type
// ============================================================

impl<E> Handled<E> {
    /// Create a new Handled wrapper around an error.
    /// Message is computed lazily on first access.
    #[cfg(feature = "std")]
    #[inline]
    pub fn new(source: E) -> Self
    where
        E: fmt::Display,
    {
        Self {
            source,
            message: OnceLock::new(),
            locations: LocationVec::new(),
            contexts: None,
            chained: None,
        }
    }

    #[cfg(not(feature = "std"))]
    #[inline]
    pub fn new(source: E) -> Self
    where
        E: fmt::Display,
    {
        Self {
            source,
            message: OnceLock::new(),
            locations: LocationVec::new(),
            contexts: None,
        }
    }

    /// Add a frame with location information.
    /// This is cheap - just stores file:line:col, no allocation beyond Vec growth.
    #[doc(hidden)]
    #[inline]
    pub fn frame(mut self, file: &'static str, line: u32, col: u32) -> Self {
        if self.locations.len() < DEFAULT_LOCATION_LIMIT {
            self.locations.push(Location { file, line, col });
        }
        self
    }

    /// Add context message to the most recent frame.
    /// This is expensive - allocates the contexts Vec if needed.
    #[doc(hidden)]
    #[inline]
    pub fn ctx(mut self, msg: impl Into<String>) -> Self {
        let location_idx = self.locations.len().saturating_sub(1) as u16;
        let contexts = self.contexts.get_or_insert_with(Vec::new);

        if contexts.len() < DEFAULT_CONTEXT_LIMIT {
            // Check if we already have a context for this location
            if let Some(entry) = contexts.iter_mut().find(|e| e.location_idx == location_idx) {
                entry.message = Some(msg.into());
            } else {
                contexts.push(ContextEntry {
                    location_idx,
                    message: Some(msg.into()),
                    attachments: Vec::new(),
                });
            }
        }
        self
    }

    /// Add a scope frame - creates a new frame for hierarchical context.
    /// Unlike `.ctx()` which modifies the current frame, this adds a new frame.
    #[doc(hidden)]
    #[inline]
    pub fn scope(
        mut self,
        file: &'static str,
        line: u32,
        col: u32,
        msg: impl Into<String>,
    ) -> Self {
        if self.locations.len() < DEFAULT_LOCATION_LIMIT {
            self.locations.push(Location { file, line, col });
            let location_idx = (self.locations.len() - 1) as u16;

            let contexts = self.contexts.get_or_insert_with(Vec::new);
            if contexts.len() < DEFAULT_CONTEXT_LIMIT {
                contexts.push(ContextEntry {
                    location_idx,
                    message: Some(msg.into()),
                    attachments: Vec::new(),
                });
            }
        }
        self
    }

    /// Add a scope frame with key-value attachments.
    #[doc(hidden)]
    #[inline]
    pub fn scope_kv(
        mut self,
        file: &'static str,
        line: u32,
        col: u32,
        msg: impl Into<String>,
        attachments: Vec<(Cow<'static, str>, Value)>,
    ) -> Self {
        if self.locations.len() < DEFAULT_LOCATION_LIMIT {
            self.locations.push(Location { file, line, col });
            let location_idx = (self.locations.len() - 1) as u16;

            let contexts = self.contexts.get_or_insert_with(Vec::new);
            if contexts.len() < DEFAULT_CONTEXT_LIMIT {
                contexts.push(ContextEntry {
                    location_idx,
                    message: Some(msg.into()),
                    attachments,
                });
            }
        }
        self
    }

    /// Add key-value attachment to the most recent frame with typed value.
    #[doc(hidden)]
    #[inline]
    pub fn kv(mut self, key: &'static str, val: impl IntoValue) -> Self {
        let location_idx = self.locations.len().saturating_sub(1) as u16;
        let contexts = self.contexts.get_or_insert_with(Vec::new);

        // Find or create context entry for this location
        if let Some(entry) = contexts.iter_mut().find(|e| e.location_idx == location_idx) {
            entry.attachments.push((Cow::Borrowed(key), val.into_value()));
        } else if contexts.len() < DEFAULT_CONTEXT_LIMIT {
            contexts.push(ContextEntry {
                location_idx,
                message: None,
                attachments: vec![(Cow::Borrowed(key), val.into_value())],
            });
        }
        self
    }

    /// Get the error message, computing it lazily on first access.
    pub fn message(&self) -> &str
    where
        E: fmt::Display,
    {
        self.message.get_or_init(|| self.source.to_string())
    }

    /// Try to get a reference to a specific error type.
    ///
    /// This works with both concrete error types (direct access) and
    /// type-erased errors (runtime downcasting).
    #[doc(hidden)]
    #[inline]
    pub fn try_catch<Target>(&self) -> Option<&Target>
    where
        E: TryCatch<Target>,
    {
        self.source.try_catch()
    }

    /// Get the underlying error source.
    pub fn source_ref(&self) -> &E {
        &self.source
    }

    /// Consume and return the underlying error.
    pub fn into_source(self) -> E {
        self.source
    }

    /// Iterate over frames in the trace.
    /// Combines locations with their optional contexts.
    pub fn frames(&self) -> impl Iterator<Item = FrameView<'_>> {
        let contexts = self.contexts.as_ref();
        self.locations.iter().enumerate().map(move |(idx, loc)| {
            let idx = idx as u16;
            let ctx = contexts.and_then(|c| c.iter().find(|e| e.location_idx == idx));
            FrameView {
                file: loc.file,
                line: loc.line,
                col: loc.col,
                context: ctx.and_then(|c| c.message.as_deref()),
                attachments_inner: ctx.map(|c| c.attachments.as_slice()).unwrap_or(&[]),
            }
        })
    }

    /// Number of location frames in the trace.
    pub fn depth(&self) -> usize {
        self.locations.len()
    }

    /// Whether the trace is empty.
    pub fn is_empty(&self) -> bool {
        self.locations.is_empty()
    }

    /// Number of context entries (frames with messages/attachments).
    pub fn context_count(&self) -> usize {
        self.contexts.as_ref().map(|c| c.len()).unwrap_or(0)
    }

    /// Add a frame at the caller's location.
    #[track_caller]
    pub fn here(self) -> Self {
        let loc = core::panic::Location::caller();
        self.frame(loc.file(), loc.line(), loc.column())
    }

    /// Convert to a type-erased Handled.
    #[cfg(feature = "std")]
    pub fn erase(self) -> Handled<Error>
    where
        E: StdError + Send + Sync + 'static,
    {
        Handled {
            source: Error::new(self.source),
            message: self.message,
            locations: self.locations,
            contexts: self.contexts,
            chained: self.chained,
        }
    }

    /// Map the error type while preserving context.
    pub fn map_err<F, O>(self, f: F) -> Handled<O>
    where
        F: FnOnce(E) -> O,
        O: fmt::Display,
    {
        let new_source = f(self.source);
        Handled {
            source: new_source,
            message: OnceLock::new(),  // Lazy - will compute from new source
            locations: self.locations,
            contexts: self.contexts,
            #[cfg(feature = "std")]
            chained: self.chained,
        }
    }
}

// ============================================================
// Handled<Error> specific methods (type-erased)
// ============================================================

impl Handled<Error> {
    /// Wrap any error, avoiding double-boxing if already Handled.
    /// Message is computed lazily on first access.
    #[cfg(feature = "std")]
    #[inline]
    pub fn wrap<E>(e: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        use core::any::TypeId;
        if TypeId::of::<E>() == TypeId::of::<Self>() {
            // SAFETY: TypeId guarantees E is Handled. We read it out and forget
            // the original to avoid double-drop.
            unsafe {
                let handled = core::ptr::read(&e as *const E as *const Self);
                core::mem::forget(e);
                handled
            }
        } else {
            Self {
                source: Error::new(e),
                message: OnceLock::new(),
                locations: LocationVec::new(),
                contexts: None,
                chained: None,
            }
        }
    }

    /// Wrap a boxed error into a type-erased Handled.
    /// If the boxed error is already a Handled, unwrap it to avoid double-wrapping.
    /// Message is computed lazily on first access.
    #[cfg(feature = "std")]
    #[inline]
    pub fn wrap_box(e: Box<dyn StdError + Send + Sync + 'static>) -> Self {
        // Check if it's already a Handled<Error>
        match e.downcast::<Self>() {
            Ok(handled) => *handled,
            Err(e) => {
                Self {
                    source: Error::from_box(e),
                    message: OnceLock::new(),
                    locations: LocationVec::new(),
                    contexts: None,
                    chained: None,
                }
            }
        }
    }

    /// Wrap an Error directly.
    /// Message is computed lazily on first access.
    #[cfg(feature = "std")]
    #[inline]
    pub fn wrap_erased(e: Error) -> Self {
        Self {
            source: e,
            message: OnceLock::new(),
            locations: LocationVec::new(),
            contexts: None,
            chained: None,
        }
    }

    /// Wrap a boxed error and add a frame in one operation.
    /// Equivalent to wrap_box().frame() but as a single call.
    #[doc(hidden)]
    #[cfg(feature = "std")]
    #[inline]
    pub fn wrap_box_with_frame(
        e: Box<dyn StdError + Send + Sync + 'static>,
        file: &'static str,
        line: u32,
        col: u32,
    ) -> Self {
        Self::wrap_box(e).frame(file, line, col)
    }

    #[cfg(not(feature = "std"))]
    #[inline]
    pub fn wrap<E>(e: E) -> Self
    where
        E: fmt::Debug + fmt::Display + Send + Sync + 'static,
    {
        use core::any::TypeId;
        if TypeId::of::<E>() == TypeId::of::<Self>() {
            unsafe {
                let handled = core::ptr::read(&e as *const E as *const Self);
                core::mem::forget(e);
                handled
            }
        } else {
            Self {
                source: Error::new(e),
                message: OnceLock::new(),
                locations: LocationVec::new(),
                contexts: None,
            }
        }
    }

    /// Create from a message string.
    /// Message is pre-initialized since we already have it.
    #[cfg(feature = "std")]
    #[inline]
    pub fn msg(message: impl Into<String>) -> Self {
        let message = message.into();
        let msg_lock = OnceLock::new();
        let _ = msg_lock.set(message.clone());
        Self {
            source: Error::new(StringError(message)),
            message: msg_lock,
            locations: LocationVec::new(),
            contexts: None,
            chained: None,
        }
    }

    #[cfg(not(feature = "std"))]
    #[inline]
    pub fn msg(message: impl Into<String>) -> Self {
        let message = message.into();
        let msg_lock = OnceLock::new();
        let _ = msg_lock.set(message.clone());
        Self {
            source: Error::new(StringError(message)),
            message: msg_lock,
            locations: LocationVec::new(),
            contexts: None,
        }
    }

    /// Chain this error after a previous error.
    ///
    /// Used by `try any` to link all failed attempts together so that
    /// `catch any`, `throw any`, and `inspect any` can find errors
    /// from any iteration.
    ///
    /// The previous error becomes accessible via `chain_any`/`chain_all`.
    /// The root error type is preserved for `catch Type` matching.
    #[doc(hidden)]
    #[cfg(feature = "std")]
    pub fn chain_after(mut self, previous: Self) -> Self {
        // Flatten any existing chain from self
        let existing_chain = self.chained.take();

        // Build chain: previous -> existing_chain (if any)
        let new_previous = if let Some(existing) = existing_chain {
            // previous's chain gets extended with existing
            let mut prev = previous;
            prev.chained = Some(existing);
            prev
        } else {
            previous
        };

        self.chained = Some(Box::new(new_previous));
        self
    }

    /// Get the root error as a trait object.
    #[cfg(feature = "std")]
    pub fn root(&self) -> &(dyn StdError + 'static) {
        self.source.as_dyn_error()
    }

    /// Try to downcast to a specific error type.
    #[cfg(feature = "std")]
    #[inline]
    pub fn downcast_ref<T: StdError + 'static>(&self) -> Option<&T> {
        self.source.downcast_ref::<T>()
    }

    /// Try to downcast and consume the error.
    #[cfg(feature = "std")]
    #[inline]
    pub fn downcast<T: StdError + 'static>(self) -> core::result::Result<T, Self> {
        if self.source.downcast_ref::<T>().is_some() {
            let Self {
                source,
                locations,
                contexts,
                message,
                chained,
            } = self;
            match source.downcast::<T>() {
                Ok(e) => Ok(e),
                Err(source) => Err(Self {
                    source,
                    locations,
                    contexts,
                    message,
                    chained,
                }),
            }
        } else {
            Err(self)
        }
    }

    /// Find the first error of type `T` in the cause chain.
    ///
    /// Walks the error chain via `std::error::Error::source()` and returns
    /// a reference to the first error that matches type `T`.
    ///
    /// # Example
    ///
    /// ```
    /// use handle_this::{Handled, Result};
    /// use std::io;
    ///
    /// fn check_chain(err: &Handled) {
    ///     if let Some(io_err) = err.chain_any::<io::Error>() {
    ///         println!("Found IO error in chain: {:?}", io_err.kind());
    ///     }
    /// }
    /// ```
    #[cfg(feature = "std")]
    pub fn chain_any<T: StdError + 'static>(&self) -> Option<&T> {
        // First check the root error
        if let Some(e) = self.source.downcast_ref::<T>() {
            return Some(e);
        }

        // Walk the source's cause chain (for wrapped errors with causes)
        let mut current: Option<&(dyn StdError + 'static)> = self.source.as_dyn_error().source();
        while let Some(err) = current {
            // Direct type match
            if let Some(e) = err.downcast_ref::<T>() {
                return Some(e);
            }

            // If it's a Handled<Error>, recursively search inside it
            if let Some(handled) = err.downcast_ref::<Handled<Error>>() {
                if let Some(e) = handled.chain_any::<T>() {
                    return Some(e);
                }
            }

            current = err.source();
        }

        // Check the chained previous errors (from chain_after)
        if let Some(ref chained) = self.chained {
            if let Some(e) = chained.chain_any::<T>() {
                return Some(e);
            }
        }

        None
    }

    /// Find all errors of type `T` in the cause chain.
    ///
    /// Walks the error chain via `std::error::Error::source()` and collects
    /// references to all errors that match type `T`.
    ///
    /// # Example
    ///
    /// ```
    /// use handle_this::{Handled, Result};
    /// use std::io;
    ///
    /// fn check_all(err: &Handled) {
    ///     let io_errors = err.chain_all::<io::Error>();
    ///     for e in io_errors {
    ///         println!("IO error: {:?}", e.kind());
    ///     }
    /// }
    /// ```
    #[cfg(feature = "std")]
    pub fn chain_all<T: StdError + 'static>(&self) -> Vec<&T> {
        let mut matches = Vec::new();

        // First check the root error
        if let Some(e) = self.source.downcast_ref::<T>() {
            matches.push(e);
        }

        // Walk the source's cause chain (for wrapped errors with causes)
        let mut current: Option<&(dyn StdError + 'static)> = self.source.as_dyn_error().source();
        while let Some(err) = current {
            // If it's a Handled<Error>, recursively search inside it
            if let Some(handled) = err.downcast_ref::<Handled<Error>>() {
                matches.extend(handled.chain_all::<T>());
                break; // Handled contains everything, nothing more to walk
            }

            // Direct type match
            if let Some(e) = err.downcast_ref::<T>() {
                matches.push(e);
            }

            current = err.source();
        }

        // Check the chained previous errors (from chain_after)
        if let Some(ref chained) = self.chained {
            matches.extend(chained.chain_all::<T>());
        }

        matches
    }
}

// Note: We intentionally do NOT have a generic `From<E> for Handled<E>` impl
// because it would conflict with `From<T> for T` when E is already a Handled.
// Instead, the __try_block! macro produces the raw error type, and wrapping
// happens at the macro boundary.

// ============================================================
// Display and Error implementations
// ============================================================

impl<E: fmt::Display> fmt::Display for Handled<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = self.message.get_or_init(|| self.source.to_string());
        writeln!(f, "{}", msg)?;

        if !self.locations.is_empty() {
            writeln!(f, "\nTrace (most recent last):")?;
            for (idx, loc) in self.locations.iter().enumerate() {
                write!(f, "  {}:{}:{}", loc.file, loc.line, loc.col)?;

                // Find context for this location if any
                if let Some(contexts) = &self.contexts {
                    if let Some(ctx) = contexts.iter().find(|c| c.location_idx == idx as u16) {
                        if let Some(msg) = &ctx.message {
                            write!(f, "\n    \u{2192} {}", msg)?;
                        }
                        for (k, v) in &ctx.attachments {
                            write!(f, "\n    {}: {}", k, v)?;
                        }
                    }
                }
                writeln!(f)?;
            }
        }

        Ok(())
    }
}

// Note: We intentionally do NOT have a generic `impl<E: Error> Error for Handled<E>`.
// This prevents `Handled<E>` from satisfying the `E: Error` bound in `IntoHandled`,
// which allows us to have non-conflicting trait impls for wrapping.

// StdError impl ONLY for type-erased Handled (needed for ? operator in functions returning Result<_, Handled>)
#[cfg(feature = "std")]
impl StdError for Handled<Error> {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_dyn_error())
    }
}

// ============================================================
// wrap_any - wraps errors avoiding double-wrap using TypeId
// ============================================================

/// Wrap any error in Handled, avoiding double-wrapping.
/// Uses TypeId to detect if the input is already a Handled<Error>.
#[doc(hidden)]
#[cfg(feature = "std")]
pub fn __wrap_any<E: StdError + Send + Sync + 'static>(e: E) -> Handled<Error> {
    use core::any::TypeId;
    // Check if E is Handled<Error>
    if TypeId::of::<E>() == TypeId::of::<Handled<Error>>() {
        // SAFETY: TypeId guarantees E is Handled<Error>
        unsafe {
            let handled = core::ptr::read(&e as *const E as *const Handled<Error>);
            core::mem::forget(e);
            handled
        }
    } else {
        Handled::wrap(e)
    }
}

// ============================================================
// From impls for type-erased Handled
// ============================================================

impl From<&str> for Handled<Error> {
    fn from(s: &str) -> Self {
        Self::msg(s)
    }
}

impl From<String> for Handled<Error> {
    fn from(s: String) -> Self {
        Self::msg(s)
    }
}

// Primitive type impls - allow using numbers as error codes
macro_rules! impl_from_primitive {
    ($($t:ty),*) => {
        $(
            impl From<$t> for Handled<Error> {
                fn from(v: $t) -> Self {
                    Self::msg(v.to_string())
                }
            }
        )*
    };
}

impl_from_primitive!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

// Unit type - used as default error type for try catch inference
impl From<()> for Handled<Error> {
    fn from(_: ()) -> Self {
        Self::msg("error")
    }
}

// Box<dyn Display> - allows any Display type to convert to Handled
#[cfg(feature = "std")]
impl From<Box<dyn core::fmt::Display + Send + Sync>> for Handled<Error> {
    fn from(e: Box<dyn core::fmt::Display + Send + Sync>) -> Self {
        Self::msg(e.to_string())
    }
}

#[cfg(feature = "std")]
impl From<Box<dyn StdError + Send + Sync + 'static>> for Handled<Error> {
    fn from(e: Box<dyn StdError + Send + Sync + 'static>) -> Self {
        Self::wrap_box(e)
    }
}

// Specific From impls for common error types.
// We can't use a blanket impl because Handled<Error> itself implements Error,
// which would conflict with From<T> for T.

#[cfg(feature = "std")]
impl From<std::io::Error> for Handled<Error> {
    fn from(e: std::io::Error) -> Self {
        Self::wrap(e)
    }
}

#[cfg(feature = "std")]
impl From<std::num::ParseIntError> for Handled<Error> {
    fn from(e: std::num::ParseIntError) -> Self {
        Self::wrap(e)
    }
}

#[cfg(feature = "std")]
impl From<std::num::ParseFloatError> for Handled<Error> {
    fn from(e: std::num::ParseFloatError) -> Self {
        Self::wrap(e)
    }
}

#[cfg(feature = "std")]
impl From<std::str::Utf8Error> for Handled<Error> {
    fn from(e: std::str::Utf8Error) -> Self {
        Self::wrap(e)
    }
}

#[cfg(feature = "std")]
impl From<std::string::FromUtf8Error> for Handled<Error> {
    fn from(e: std::string::FromUtf8Error) -> Self {
        Self::wrap(e)
    }
}

// Note: We intentionally don't have a generic From<Handled<E>> for Handled<Error>
// as it conflicts with the blanket From<T> for T, causing type inference issues.
// Use the .erase() method for explicit conversion instead.

// ============================================================
// anyhow interop
// ============================================================

#[cfg(feature = "anyhow")]
impl From<anyhow::Error> for Handled<Error> {
    fn from(e: anyhow::Error) -> Self {
        match e.downcast::<Handled<Error>>() {
            Ok(h) => h,
            Err(e) => {
                let msg = e.to_string();
                let mut handled = Self::msg(msg);
                for (i, cause) in e.chain().skip(1).enumerate() {
                    handled = handled
                        .frame("<anyhow>", i as u32, 0)
                        .ctx(cause.to_string());
                }
                handled
            }
        }
    }
}

// ============================================================
// eyre interop
// ============================================================

#[cfg(feature = "eyre")]
impl From<eyre::Report> for Handled<Error> {
    fn from(e: eyre::Report) -> Self {
        match e.downcast::<Handled<Error>>() {
            Ok(h) => h,
            Err(e) => {
                let msg = e.to_string();
                let mut handled = Self::msg(msg);
                for (i, cause) in e.chain().skip(1).enumerate() {
                    handled = handled
                        .frame("<eyre>", i as u32, 0)
                        .ctx(cause.to_string());
                }
                handled
            }
        }
    }
}

// ============================================================
// Serde support
// ============================================================

#[cfg(feature = "serde")]
mod serde_impl {
    use super::*;
    #[cfg(feature = "std")]
    use std::collections::BTreeMap;
    #[cfg(not(feature = "std"))]
    use alloc::collections::BTreeMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    // Serialize Value to preserve type information
    impl Serialize for Value {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            match self {
                Value::String(s) => serializer.serialize_str(s),
                Value::Int(n) => serializer.serialize_i64(*n),
                Value::Uint(n) => serializer.serialize_u64(*n),
                Value::Float(n) => serializer.serialize_f64(*n),
                Value::Bool(b) => serializer.serialize_bool(*b),
                Value::Null => serializer.serialize_none(),
            }
        }
    }

    impl<'de> Deserialize<'de> for Value {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            use serde::de::{self, Visitor};

            struct ValueVisitor;

            impl<'de> Visitor<'de> for ValueVisitor {
                type Value = Value;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("a string, number, boolean, or null")
                }

                fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
                    Ok(Value::Bool(v))
                }

                fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
                    Ok(Value::Int(v))
                }

                fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
                    Ok(Value::Uint(v))
                }

                fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
                    Ok(Value::Float(v))
                }

                fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
                    Ok(Value::String(v.to_string()))
                }

                fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
                    Ok(Value::String(v))
                }

                fn visit_none<E: de::Error>(self) -> Result<Value, E> {
                    Ok(Value::Null)
                }

                fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
                    Ok(Value::Null)
                }
            }

            deserializer.deserialize_any(ValueVisitor)
        }
    }

    #[derive(Serialize, Deserialize)]
    struct SerializedFrame {
        file: String,
        line: u32,
        col: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        attachments: BTreeMap<String, Value>,
    }

    #[derive(Serialize, Deserialize)]
    struct SerializedHandled {
        message: String,
        trace: Vec<SerializedFrame>,
    }

    // Only implement for Error variant (type-erased)
    impl Serialize for Handled<Error> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let contexts = self.contexts.as_ref();
            let serialized = SerializedHandled {
                message: self.message().to_string(),
                trace: self
                    .locations
                    .iter()
                    .enumerate()
                    .map(|(idx, loc)| {
                        let ctx = contexts.and_then(|c| c.iter().find(|e| e.location_idx == idx as u16));
                        SerializedFrame {
                            file: loc.file.to_string(),
                            line: loc.line,
                            col: loc.col,
                            message: ctx.and_then(|c| c.message.clone()),
                            attachments: ctx
                                .map(|c| c.attachments.iter()
                                    .map(|(k, v)| (k.to_string(), v.clone()))
                                    .collect::<BTreeMap<_, _>>())
                                .unwrap_or_default(),
                        }
                    })
                    .collect(),
            };
            serialized.serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for Handled<Error> {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let serialized = SerializedHandled::deserialize(deserializer)?;

            let mut locations = LocationVec::new();
            let mut contexts = Vec::new();

            for (idx, f) in serialized.trace.into_iter().enumerate() {
                // Note: deserialized files are owned strings, we leak them to get 'static
                // This is acceptable for deserialized errors which are typically short-lived
                let file: &'static str = Box::leak(f.file.into_boxed_str());
                locations.push(Location {
                    file,
                    line: f.line,
                    col: f.col,
                });

                if f.message.is_some() || !f.attachments.is_empty() {
                    contexts.push(ContextEntry {
                        location_idx: idx as u16,
                        message: f.message,
                        attachments: f.attachments
                            .into_iter()
                            .map(|(k, v)| (Cow::Owned(k), v))
                            .collect(),
                    });
                }
            }

            Ok(Self {
                message: {
                    let lock = OnceLock::new();
                    let _ = lock.set(serialized.message.clone());
                    lock
                },
                source: Error::new(StringError(serialized.message)),
                locations,
                contexts: if contexts.is_empty() { None } else { Some(contexts) },
                #[cfg(feature = "std")]
                chained: None,
            })
        }
    }

    impl Serialize for Location {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            SerializedFrame {
                file: self.file.to_string(),
                line: self.line,
                col: self.col,
                message: None,
                attachments: BTreeMap::new(),
            }
            .serialize(serializer)
        }
    }

    // Custom serializer for FrameView that serializes attachments as a map
    impl Serialize for FrameView<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeStruct;
            let mut state = serializer.serialize_struct("FrameView", 5)?;
            state.serialize_field("file", self.file)?;
            state.serialize_field("line", &self.line)?;
            state.serialize_field("col", &self.col)?;
            if self.context.is_some() {
                state.serialize_field("message", &self.context)?;
            }
            if !self.attachments_inner.is_empty() {
                // Serialize as a map
                let map: BTreeMap<&str, &Value> = self.attachments_inner
                    .iter()
                    .map(|(k, v)| (k.as_ref(), v))
                    .collect();
                state.serialize_field("attachments", &map)?;
            }
            state.end()
        }
    }
}
