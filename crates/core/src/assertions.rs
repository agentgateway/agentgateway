// Helper functions for checking type sizes. They are primarily useful for
// futures whose concrete types cannot be named directly. Future size is
// determined by the largest suspend state, so deeply branched async code can
// quietly grow connection/task memory and stack pressure.

#[inline(always)]
pub fn size_at_most<const MAX: usize, T>(t: T) -> T {
	SizeAtMost::<MAX>::check(t)
}

pub struct SizeAtMost<const MAX: usize>;

impl<const MAX: usize> SizeAtMost<MAX> {
	#[inline(always)]
	pub fn check<T>(t: T) -> T {
		#[cfg(not(feature = "assert_size_runtime"))]
		const {
			assert!(
				std::mem::size_of::<T>() <= MAX,
				"type size exceeds assert_size limit"
			);
		}
		#[cfg(feature = "assert_size_runtime")]
		assert!(
			std::mem::size_of::<T>() <= MAX,
			"type size {} exceeds maximum {}",
			std::mem::size_of::<T>(),
			MAX
		);
		t
	}
}

pub trait AssertSize: Sized {
	#[inline(always)]
	fn assert_size<const MAX: usize>(self) -> Self {
		SizeAtMost::<MAX>::check(self)
	}
}

impl<T> AssertSize for T {}
