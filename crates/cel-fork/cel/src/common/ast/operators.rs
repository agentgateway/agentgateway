pub fn find_operator(input: &str) -> Option<&str> {
    for (op, operator) in OPERATORS {
        if op == input {
            return Some(operator);
        }
    }
    None
}

pub const CONDITIONAL: &str = "_?_:_";
pub const LOGICAL_AND: &str = "_&&_";
pub const LOGICAL_OR: &str = "_||_";
pub const LOGICAL_NOT: &str = "!_";
pub const SUBSTRACT: &str = "_-_";
pub const ADD: &str = "_+_";
pub const MULTIPLY: &str = "_*_";
pub const DIVIDE: &str = "_/_";
pub const MODULO: &str = "_%_";
pub const EQUALS: &str = "_==_";
pub const NOT_EQUALS: &str = "_!=_";
pub const GREATER_EQUALS: &str = "_>=_";
pub const LESS_EQUALS: &str = "_<=_";
pub const GREATER: &str = "_>_";
pub const LESS: &str = "_<_";
pub const NEGATE: &str = "-_";
pub const INDEX: &str = "_[_]";
pub const OPT_INDEX: &str = "_[?_]";
pub const OPT_SELECT: &str = "_?._";

pub const EXISTS_ONE: &str = "exists_one";
pub const HAS: &str = "has";
pub const ALL: &str = "all";
pub const EXISTS: &str = "exists";
pub const MAP: &str = "map";
pub const FILTER: &str = "filter";

pub const NOT_STRICTLY_FALSE: &str = "@not_strictly_false";
pub const IN: &str = "@in";

const OPERATORS: [(&str, &str); 12] = [
    ("-", SUBSTRACT),
    ("+", ADD),
    ("*", MULTIPLY),
    ("/", DIVIDE),
    ("%", MODULO),
    ("==", EQUALS),
    ("!=", NOT_EQUALS),
    (">=", GREATER_EQUALS),
    ("<=", LESS_EQUALS),
    (">", GREATER),
    ("<", LESS),
    ("in", IN),
];
