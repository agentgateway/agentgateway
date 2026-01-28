use std::collections::HashSet;

use crate::common::ast::{Expr, IdedExpr};

/// A collection of all the references that an expression makes to variables and functions.
pub struct ExpressionReferences<'expr> {
    variables: HashSet<&'expr str>,
    functions: HashSet<&'expr str>,
}

impl ExpressionReferences<'_> {
    /// Returns true if the expression references the provided variable name.
    ///
    /// # Example
    /// ```rust
    /// # use cel::parser::Parser;
    /// let expression = Parser::new().parse("foo.bar == true").unwrap();
    /// let references = expression.references();
    /// assert!(references.has_variable("foo"));
    /// ```
    pub fn has_variable(&self, name: impl AsRef<str>) -> bool {
        self.variables.contains(name.as_ref())
    }

    /// Returns true if the expression references the provided function name.
    ///
    /// # Example
    /// ```rust
    /// # use cel::parser::Parser;
    /// let expression = Parser::new().parse("size(foo) > 0").unwrap();
    /// let references = expression.references();
    /// assert!(references.has_function("size"));
    /// ```
    pub fn has_function(&self, name: impl AsRef<str>) -> bool {
        self.functions.contains(name.as_ref())
    }

    /// Returns a list of all variables referenced in the expression.
    ///
    /// # Example
    /// ```rust
    /// # use cel::parser::Parser;
    /// let expression = Parser::new().parse("foo.bar == true").unwrap();
    /// let references = expression.references();
    /// assert_eq!(vec!["foo"], references.variables());
    /// ```
    pub fn variables(&self) -> Vec<&str> {
        self.variables.iter().copied().collect()
    }

    /// Returns a list of all functions referenced in the expression.
    ///
    /// # Example
    /// ```rust
    /// # use cel::parser::Parser;
    /// let expression = Parser::new().parse("size(foo) > 0").unwrap();
    /// let references = expression.references();
    /// assert!(references.functions().contains(&"_>_"));
    /// assert!(references.functions().contains(&"size"));
    /// ```
    pub fn functions(&self) -> Vec<&str> {
        self.functions.iter().copied().collect()
    }
}

impl IdedExpr {
    /// Returns a set of all variables and functions referenced in the expression.
    ///
    /// # Example
    /// ```rust
    /// # use cel::parser::Parser;
    /// let expression = Parser::new().parse("foo && size(foo) > 0").unwrap();
    /// let references = expression.references();
    ///
    /// assert!(references.has_variable("foo"));
    /// assert!(references.has_function("size"));
    /// ```
    pub fn references(&self) -> ExpressionReferences<'_> {
        let mut variables = HashSet::new();
        let mut functions = HashSet::new();
        self._references(&mut variables, &mut functions);
        ExpressionReferences {
            variables,
            functions,
        }
    }

    /// Internal recursive function to collect all variable and function references in the expression.
    fn _references<'expr>(
        &'expr self,
        variables: &mut HashSet<&'expr str>,
        functions: &mut HashSet<&'expr str>,
    ) {
        match &self.expr {
            Expr::Optimized { original, .. } => {
                original._references(variables, functions);
            }
            Expr::Unspecified => {}
            Expr::Call(call) => {
                functions.insert(&call.func_name);
                if let Some(target) = &call.target {
                    target._references(variables, functions);
                }
                for arg in &call.args {
                    arg._references(variables, functions);
                }
            }
            Expr::Comprehension(comp) => {
                comp.iter_range._references(variables, functions);
                comp.accu_init._references(variables, functions);
                comp.loop_cond._references(variables, functions);
                comp.loop_step._references(variables, functions);
                comp.result._references(variables, functions);
            }
            Expr::Ident(name) => {
                // todo! Might want to make this "smarter" (are we in a comprehension?) and better encode these in const
                if !name.starts_with('@') {
                    variables.insert(name);
                }
            }
            Expr::List(list) => {
                for elem in &list.elements {
                    elem._references(variables, functions);
                }
            }
            Expr::Literal(_) => {}
            Expr::Inline(_) => {}
            Expr::Map(map) => {
                for entry in &map.entries {
                    match &entry.expr {
                        crate::common::ast::EntryExpr::StructField(field) => {
                            field.value._references(variables, functions);
                        }
                        crate::common::ast::EntryExpr::MapEntry(map_entry) => {
                            map_entry.key._references(variables, functions);
                            map_entry.value._references(variables, functions);
                        }
                    }
                }
            }
            Expr::Select(select) => {
                select.operand._references(variables, functions);
            }
            Expr::Struct(struct_expr) => {
                for entry in &struct_expr.entries {
                    match &entry.expr {
                        crate::common::ast::EntryExpr::StructField(field) => {
                            field.value._references(variables, functions);
                        }
                        crate::common::ast::EntryExpr::MapEntry(map_entry) => {
                            map_entry.key._references(variables, functions);
                            map_entry.value._references(variables, functions);
                        }
                    }
                }
            }
        }
    }
}
