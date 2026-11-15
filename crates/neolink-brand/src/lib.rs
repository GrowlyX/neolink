//! Neolink brand primitives for native apps.
//!
//! * [`logo`]  the mark, wordmark and lockup as computed vector geometry.
//! * [`theme`] the design tokens: colors per mode, radii, spacing, type.
//!
//! The crate has no dependencies and knows nothing about any UI framework, so
//! it compiles everywhere and never breaks when the UI framework changes. The
//! app adapts it at the edge: see `AGENTS.md` in the kit for the GPUI mapping.

pub mod logo;
pub mod theme;
