/// ADDER_TYPE types provide a '+' operator overload.
pub const ADDER_TYPE: u16 = 1;

/// COMPARER_TYPE types support ordering comparisons '<', '<=', '>', '>='.
pub const COMPARER_TYPE: u16 = ADDER_TYPE << 1;

/// CONTAINER_TYPE types support 'in' operations.
pub const CONTAINER_TYPE: u16 = COMPARER_TYPE << 1;

/// DIVIDER_TYPE types support '/' operations.
pub const DIVIDER_TYPE: u16 = CONTAINER_TYPE << 1;

/// FIELD_TESTER_TYPE types support the detection of field value presence.
pub const FIELD_TESTER_TYPE: u16 = DIVIDER_TYPE << 1;

/// INDEXER_TYPE types support index access with dynamic values.
pub const INDEXER_TYPE: u16 = FIELD_TESTER_TYPE << 1;

/// ITERABLE_TYPE types can be iterated over in comprehensions.
pub const ITERABLE_TYPE: u16 = INDEXER_TYPE << 1;

/// ITERATOR_TYPE types support iterator semantics.
pub const ITERATOR_TYPE: u16 = ITERABLE_TYPE << 1;

/// MATCHER_TYPE types support pattern matching via 'matches' method.
pub const MATCHER_TYPE: u16 = ITERATOR_TYPE << 1;

/// MODDER_TYPE types support modulus operations '%'
pub const MODDER_TYPE: u16 = MATCHER_TYPE << 1;

/// MULTIPLIER_TYPE types support '*' operations.
pub const MULTIPLIER_TYPE: u16 = MODDER_TYPE << 1;

/// NEGATOR_TYPE types support either negation via '!' or '-'
pub const NEGATOR_TYPE: u16 = MULTIPLIER_TYPE << 1;

/// RECEIVER_TYPE types support dynamic dispatch to instance methods.
pub const RECEIVER_TYPE: u16 = NEGATOR_TYPE << 1;

/// SIZER_TYPE types support the size() method.
pub const SIZER_TYPE: u16 = RECEIVER_TYPE << 1;

/// SUBTRACTOR_TYPE types support '-' operations.
pub const SUBTRACTOR_TYPE: u16 = SIZER_TYPE << 1;

/// FOLDABLE_TYPE types support comprehensions v2 macros which iterate over (key, value) pairs.
pub const FOLDABLE_TYPE: u16 = SUBTRACTOR_TYPE << 1;
