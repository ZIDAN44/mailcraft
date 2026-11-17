//! Last-resort guards at non-unwinding platform callback boundaries.

/// Catch a panic before it escapes a JNI callback, where Rust cannot unwind into Java.
#[cfg(any(target_os = "android", test))]
pub(crate) fn catch_or<T>(operation: impl FnOnce() -> T, fallback: T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_uses_the_callback_fallback() {
        let result = catch_or(|| std::panic::resume_unwind(Box::new(())), 1);
        assert_eq!(result, 1);
    }
}
