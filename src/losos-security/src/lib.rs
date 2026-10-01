//! The checks, as a library, so `tests/` can drive them directly.
//!
//! The binary is a thin shell over this: the interesting part is a pure
//! function from a directory tree to a list of verdicts, and that is worth
//! testing without a process boundary in the way.
pub mod attr;
pub mod checks;
