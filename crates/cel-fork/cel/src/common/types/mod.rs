use crate::common::traits;

mod bool;
mod bytes;
mod double;
mod duration;
mod int;
mod null;
mod optional;
mod string;
mod timestamp;
mod uint;

pub use bool::Bool;
pub use bytes::Bytes;
pub use double::Double;
pub use duration::Duration;
pub use int::Int;
pub use null::Null;
pub use string::String;
pub use timestamp::Timestamp;
pub use uint::UInt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Unspecified,
    Error,
    Dyn,
    Any,
    Boolean,
    Bytes,
    Double,
    Duration,
    Int,
    List,
    Map,
    NullType,
    Opaque,
    String,
    Struct,
    Timestamp,
    Type,
    TypeParam,
    UInt,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Type<'a> {
    kind: Kind,
    parameters: &'a [&'a Type<'a>],
    runtime_type_name: &'a str,
    trait_mask: u16,
}

pub const ANY_TYPE: Type = Type {
    kind: Kind::Any,
    parameters: &[],
    runtime_type_name: "google.protobuf.Any",
    trait_mask: traits::FIELD_TESTER_TYPE | traits::INDEXER_TYPE,
};

pub const BOOL_TYPE: Type = Type {
    kind: Kind::Boolean,
    parameters: &[],
    runtime_type_name: "bool",
    trait_mask: traits::COMPARER_TYPE | traits::NEGATOR_TYPE,
};

pub const BYTES_TYPE: Type = Type {
    kind: Kind::Bytes,
    parameters: &[],
    runtime_type_name: "bytes",
    trait_mask: traits::ADDER_TYPE | traits::COMPARER_TYPE | traits::SIZER_TYPE,
};

pub const DOUBLE_TYPE: Type = Type {
    kind: Kind::Double,
    parameters: &[],
    runtime_type_name: "double",
    trait_mask: traits::ADDER_TYPE
        | traits::COMPARER_TYPE
        | traits::DIVIDER_TYPE
        | traits::MULTIPLIER_TYPE
        | traits::NEGATOR_TYPE
        | traits::SUBTRACTOR_TYPE,
};

pub const DURATION_TYPE: Type = Type {
    kind: Kind::Duration,
    parameters: &[],
    runtime_type_name: "google.protobuf.Duration",
    trait_mask: traits::ADDER_TYPE
        | traits::COMPARER_TYPE
        | traits::NEGATOR_TYPE
        | traits::RECEIVER_TYPE
        | traits::SUBTRACTOR_TYPE,
};

pub const DYN_TYPE: Type = Type::simple_type(Kind::Dyn, "dyn");

pub const ERROR_TYPE: Type = Type::simple_type(Kind::Error, "error");

pub const INT_TYPE: Type = Type {
    kind: Kind::Int,
    parameters: &[],
    runtime_type_name: "int",
    trait_mask: traits::ADDER_TYPE
        | traits::COMPARER_TYPE
        | traits::DIVIDER_TYPE
        | traits::MODDER_TYPE
        | traits::MULTIPLIER_TYPE
        | traits::NEGATOR_TYPE
        | traits::SUBTRACTOR_TYPE,
};

pub const LIST_TYPE: Type = Type::new_list_type(&[&DYN_TYPE]);

pub const MAP_TYPE: Type = Type::new_map_type(&[&DYN_TYPE, &DYN_TYPE]);

pub const NULL_TYPE: Type = Type::simple_type(Kind::NullType, "null_type");

pub const STRING_TYPE: Type = Type {
    kind: Kind::String,
    parameters: &[],
    runtime_type_name: "string",
    trait_mask: traits::ADDER_TYPE
        | traits::COMPARER_TYPE
        | traits::MATCHER_TYPE
        | traits::RECEIVER_TYPE
        | traits::SIZER_TYPE,
};

pub const TIMESTAMP_TYPE: Type = Type {
    kind: Kind::Timestamp,
    parameters: &[],
    runtime_type_name: "google.protobuf.Timestamp",
    trait_mask: traits::ADDER_TYPE
        | traits::COMPARER_TYPE
        | traits::RECEIVER_TYPE
        | traits::SUBTRACTOR_TYPE,
};

pub const TYPE_TYPE: Type = Type::simple_type(Kind::Type, "type");

pub const UINT_TYPE: Type = Type {
    kind: Kind::UInt,
    parameters: &[],
    runtime_type_name: "uint",
    trait_mask: traits::ADDER_TYPE
        | traits::COMPARER_TYPE
        | traits::DIVIDER_TYPE
        | traits::MODDER_TYPE
        | traits::MULTIPLIER_TYPE
        | traits::SUBTRACTOR_TYPE,
};

pub const UNKNOWN_TYPE: Type = Type::simple_type(Kind::Unknown, "unknown");

impl<'a> Type<'a> {
    pub const fn simple_type(kind: Kind, name: &str) -> Type<'_> {
        Type {
            kind,
            parameters: &[],
            runtime_type_name: name,
            trait_mask: 0,
        }
    }

    pub const fn new_list_type<'b>(param: &'b [&'b Type<'b>; 1]) -> Type<'b> {
        Type {
            kind: Kind::List,
            parameters: param,
            runtime_type_name: "list",
            trait_mask: traits::ADDER_TYPE
                | traits::CONTAINER_TYPE
                | traits::INDEXER_TYPE
                | traits::ITERABLE_TYPE
                | traits::SIZER_TYPE,
        }
    }

    pub const fn new_map_type<'b>(param: &'b [&'b Type<'b>; 2]) -> Type<'b> {
        Type {
            kind: Kind::Map,
            parameters: param,
            runtime_type_name: "map",
            trait_mask: traits::CONTAINER_TYPE
                | traits::INDEXER_TYPE
                | traits::ITERABLE_TYPE
                | traits::SIZER_TYPE,
        }
    }

    pub const fn new_unspecified_type(name: &str) -> Type<'_> {
        Type {
            kind: Kind::Unspecified,
            parameters: &[],
            runtime_type_name: name,
            trait_mask: 0,
        }
    }

    pub const fn new_opaque_type(name: &str) -> Type<'_> {
        Type {
            kind: Kind::Opaque,
            parameters: &[],
            runtime_type_name: name,
            trait_mask: 0,
        }
    }

    pub fn name(&self) -> &'a str {
        self.runtime_type_name
    }

    pub fn has_trait(&self, t: u16) -> bool {
        self.trait_mask & t == t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterized_type() {
        let param = Type {
            kind: Kind::Unspecified,
            parameters: &[],
            runtime_type_name: "",
            trait_mask: 0,
        };

        let t = std::string::String::from("List");
        let parameterized_list = Type {
            kind: Kind::List,
            parameters: &[&param],
            runtime_type_name: &t,
            trait_mask: 0,
        };
        assert_eq!(&param, parameterized_list.parameters[0]);

        let params = [&param];
        let list2 = Type::new_list_type(&params);
        assert_eq!(&param, list2.parameters[0]);
        assert_eq!(1, list2.parameters.len());

        let params = [&param, &param];
        let map = Type::new_map_type(&params);
        assert_eq!(&param, map.parameters[0]);
        assert_eq!(&param, map.parameters[1]);
        assert_eq!(2, map.parameters.len());
    }
}
