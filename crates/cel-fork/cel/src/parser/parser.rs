use std::cell::RefCell;
use std::error::Error;
use std::fmt::Display;
use std::mem;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;

use antlr4rust::common_token_stream::CommonTokenStream;
use antlr4rust::error_listener::ErrorListener;
use antlr4rust::errors::ANTLRError;
use antlr4rust::parser::ParserNodeType;
use antlr4rust::parser_rule_context::ParserRuleContext;
use antlr4rust::recognizer::Recognizer;
use antlr4rust::token::{CommonToken, Token};
use antlr4rust::token_factory::TokenFactory;
use antlr4rust::tree::{ParseTree, ParseTreeListener, ParseTreeVisitorCompat, VisitChildren};
use antlr4rust::{InputStream, Parser as AntlrParser};

use crate::common::ast;
use crate::common::ast::{
    CallExpr, EntryExpr, Expr, IdedEntryExpr, IdedExpr, ListExpr, MapEntryExpr, MapExpr,
    SelectExpr, SourceInfo, StructExpr, StructFieldExpr, operators,
};
use crate::common::value::CelVal;
use crate::parser::r#gen::{
    BoolFalseContext, BoolTrueContext, BytesContext, CELListener, CELParserContextType,
    CalcContext, CalcContextAttrs, ConditionalAndContext, ConditionalOrContext,
    ConstantLiteralContext, ConstantLiteralContextAttrs, CreateListContext, CreateMessageContext,
    CreateStructContext, DoubleContext, ExprContext, Field_initializer_listContext,
    GlobalCallContext, IdentContext, IndexContext, IndexContextAttrs, IntContext,
    ListInitContextAll, LogicalNotContext, LogicalNotContextAttrs, MapInitializerListContextAll,
    MemberCallContext, MemberCallContextAttrs, MemberExprContext, MemberExprContextAttrs,
    NegateContext, NegateContextAttrs, NestedContext, NullContext, OptFieldContextAttrs,
    PrimaryExprContext, PrimaryExprContextAttrs, RelationContext, RelationContextAttrs,
    SelectContext, SelectContextAttrs, StartContext, StartContextAttrs, StringContext, UintContext,
};
use crate::parser::{r#gen, macros, parse};

pub struct MacroExprHelper<'a> {
    helper: &'a mut ParserHelper,
    id: u64,
}

impl MacroExprHelper<'_> {
    pub fn next_expr(&mut self, expr: Expr) -> IdedExpr {
        self.helper.next_expr_for(self.id, expr)
    }

    pub(crate) fn pos_for(&self, id: u64) -> Option<(isize, isize)> {
        self.helper.source_info.pos_for(id)
    }
}

#[derive(Debug)]
pub struct ParseErrors {
    pub errors: Vec<ParseError>,
}

impl Display for ParseErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, e) in self.errors.iter().enumerate() {
            if i != 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl Error for ParseErrors {}

#[allow(dead_code)]
#[derive(Debug)]
pub struct ParseError {
    pub source: Option<Box<dyn Error + Send + Sync + 'static>>,
    pub pos: (isize, isize),
    pub msg: String,
    pub expr_id: u64,
    pub source_info: Option<Arc<SourceInfo>>,
}

impl Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ERROR: <input>:{}:{}: {}",
            self.pos.0, self.pos.1, self.msg
        )?;
        if let Some(info) = &self.source_info {
            if let Some(line) = info.snippet(self.pos.0 - 1) {
                write!(f, "\n| {line}")?;
                write!(f, "\n| {:.>width$}", "^", width = self.pos.1 as usize)?;
            }
        }
        Ok(())
    }
}

impl Error for ParseError {}

pub struct Parser {
    ast: ast::Ast,
    helper: ParserHelper,
    errors: Vec<ParseError>,
    max_recursion_depth: u16,
    enable_optional_syntax: bool,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            ast: ast::Ast {
                expr: IdedExpr::default(),
            },
            helper: ParserHelper::default(),
            errors: Vec::default(),
            max_recursion_depth: 96,
            enable_optional_syntax: false,
        }
    }

    pub fn max_recursion_depth(mut self, max: u16) -> Self {
        self.max_recursion_depth = if max == u16::MAX { max } else { max + 1 };
        self
    }

    pub fn enable_optional_syntax(mut self, enable: bool) -> Self {
        self.enable_optional_syntax = enable;
        self
    }

    fn new_logic_manager(&self, func: &str, term: IdedExpr) -> LogicManager {
        LogicManager {
            function: func.to_string(),
            terms: vec![term],
            ops: vec![],
        }
    }

    fn global_call_or_macro(
        &mut self,
        id: u64,
        func_name: String,
        args: Vec<IdedExpr>,
    ) -> IdedExpr {
        match macros::find_expander(&func_name, None, &args) {
            None => IdedExpr {
                id,
                expr: Expr::Call(CallExpr {
                    target: None,
                    func_name,
                    args,
                }),
            },
            Some(expander) => {
                let mut helper = MacroExprHelper {
                    helper: &mut self.helper,
                    id,
                };
                match expander(&mut helper, None, args) {
                    Ok(expr) => expr,
                    Err(err) => self.report_parse_error(None, err),
                }
            }
        }
    }

    fn receiver_call_or_macro(
        &mut self,
        id: u64,
        func_name: String,
        target: IdedExpr,
        args: Vec<IdedExpr>,
    ) -> IdedExpr {
        match macros::find_expander(&func_name, Some(&target), &args) {
            None => IdedExpr {
                id,
                expr: Expr::Call(CallExpr {
                    target: Some(Box::new(target)),
                    func_name,
                    args,
                }),
            },
            Some(expander) => {
                let mut helper = MacroExprHelper {
                    helper: &mut self.helper,
                    id,
                };
                match expander(&mut helper, Some(target), args) {
                    Ok(expr) => expr,
                    Err(err) => self.report_parse_error(None, err),
                }
            }
        }
    }

    pub fn parse(mut self, source: &str) -> Result<IdedExpr, ParseErrors> {
        let parse_errors = Rc::new(RefCell::new(Vec::<ParseError>::new()));
        let stream = InputStream::new(source);
        let mut lexer = r#gen::CELLexer::new(stream);
        lexer.remove_error_listeners();
        lexer.add_error_listener(Box::new(ParserErrorListener {
            parse_errors: parse_errors.clone(),
        }));

        // todo! might want to avoid this cloning here...
        self.helper.source_info.source = source.into();

        let mut prsr = r#gen::CELParser::new(CommonTokenStream::new(lexer));
        prsr.remove_error_listeners();
        prsr.add_error_listener(Box::new(ParserErrorListener {
            parse_errors: parse_errors.clone(),
        }));
        prsr.add_parse_listener(Box::new(RecursionListener {
            max: self.max_recursion_depth,
            depth: 0,
        }));
        let r = match prsr.start() {
            Ok(t) => Ok(self.visit(t.deref())),
            Err(e) => Err(ParseError {
                source: Some(Box::new(e)),
                pos: (0, 0),
                msg: "UNKNOWN".to_string(),
                expr_id: 0,
                source_info: None,
            }),
        };

        let info = self.helper.source_info;
        let source_info = Arc::new(info);

        let mut errors = parse_errors.take();
        errors.extend(self.errors);
        errors.sort_by(|a, b| a.pos.cmp(&b.pos));

        if errors.is_empty() {
            r.map_err(|e| ParseErrors { errors: vec![e] })
        } else {
            Err(ParseErrors {
                errors: errors
                    .into_iter()
                    .map(|mut e: ParseError| {
                        e.source_info = Some(source_info.clone());
                        e
                    })
                    .collect(),
            })
        }
    }

    fn field_initializer_list(
        &mut self,
        ctx: &Field_initializer_listContext<'_>,
    ) -> Vec<IdedEntryExpr> {
        let mut fields = Vec::with_capacity(ctx.fields.len());
        for (i, field) in ctx.fields.iter().enumerate() {
            if i >= ctx.cols.len() || i >= ctx.values.len() {
                return vec![];
            }
            let id = self.helper.next_id(&ctx.cols[i]);

            match field.escapeIdent() {
                None => {
                    self.report_error::<ParseError, _>(
                        field.start().deref(),
                        None,
                        "unsupported ident type",
                    );
                    continue;
                }
                Some(ident) => {
                    let field_name = ident.get_text().to_string();
                    let value = self.visit(ctx.values[i].as_ref());
                    let is_optional = match (&field.opt, self.enable_optional_syntax) {
                        (Some(opt), false) => {
                            self.report_error::<ParseError, _>(
                                opt.as_ref(),
                                None,
                                "unsupported syntax '?'",
                            );
                            continue;
                        }
                        (Some(_), true) => true,
                        (None, _) => false,
                    };
                    fields.push(IdedEntryExpr {
                        id,
                        expr: EntryExpr::StructField(StructFieldExpr {
                            field: field_name,
                            value,
                            optional: is_optional,
                        }),
                    });
                }
            }
        }
        fields
    }

    fn map_initializer_list(&mut self, ctx: &MapInitializerListContextAll) -> Vec<IdedEntryExpr> {
        if ctx.keys.is_empty() {
            return vec![];
        }
        let mut entries = Vec::with_capacity(ctx.cols.len());
        let keys = &ctx.keys;
        let vals = &ctx.values;
        for (i, col) in ctx.cols.iter().enumerate() {
            if i >= keys.len() || i >= vals.len() {
                return vec![];
            }
            let id = self.helper.next_id(col);
            let key = self.visit(keys[i].as_ref());
            let is_optional = match (keys[i].opt.as_ref(), self.enable_optional_syntax) {
                (Some(opt), false) => {
                    self.report_error::<ParseError, _>(
                        opt.as_ref(),
                        None,
                        "unsupported syntax '?'",
                    );
                    continue;
                }
                (Some(_), true) => true,
                (None, _) => false,
            };
            let value = self.visit(vals[i].as_ref());
            entries.push(IdedEntryExpr {
                id,
                expr: EntryExpr::MapEntry(MapEntryExpr {
                    key,
                    value,
                    optional: is_optional,
                }),
            })
        }
        entries
    }

    fn list_initializer_list(&mut self, ctx: &ListInitContextAll) -> (Vec<IdedExpr>, Vec<usize>) {
        let mut list = Vec::default();
        let mut optionals = Vec::default();
        for (i, e) in ctx.elems.iter().enumerate() {
            match &e.e {
                None => return (Vec::default(), Vec::default()),
                Some(exp) => {
                    if let Some(opt) = &e.opt {
                        if self.enable_optional_syntax {
                            optionals.push(i);
                        } else {
                            self.report_error::<ParseError, _>(
                                opt.as_ref(),
                                None,
                                "unsupported syntax '?'",
                            );
                            continue;
                        }
                    }
                    list.push(self.visit(exp.as_ref()));
                }
            }
        }
        (list, optionals)
    }

    fn report_error<E: Error + Send + Sync + 'static, S: Into<String>>(
        &mut self,
        token: &CommonToken,
        e: Option<E>,
        s: S,
    ) -> IdedExpr {
        let error = ParseError {
            source: e.map(|e| e.into()),
            pos: (token.line, token.column + 1),
            msg: s.into(),
            expr_id: 0,
            source_info: None,
        };
        self.report_parse_error(Some(token), error)
    }

    fn report_parse_error(&mut self, token: Option<&CommonToken>, mut e: ParseError) -> IdedExpr {
        let expr = if let Some(token) = token {
            self.helper.next_expr(token, Expr::default())
        } else {
            IdedExpr {
                id: 0,
                expr: Expr::default(),
            }
        };
        e.expr_id = expr.id;
        self.errors.push(e);
        expr
    }
}

struct RecursionListener {
    max: u16,
    depth: u16,
}

impl<'a> CELListener<'a> for RecursionListener {
    fn enter_expr(&mut self, _ctx: &ExprContext<'a>) {
        self.depth += 1;
    }

    fn exit_expr(&mut self, _ctx: &ExprContext<'a>) {
        self.depth -= 1;
    }
}

impl<'a> ParseTreeListener<'a, CELParserContextType> for RecursionListener {
    fn enter_every_rule(
        &mut self,
        ctx: &<CELParserContextType as ParserNodeType>::Type,
    ) -> Result<(), ANTLRError> {
        if self.depth > self.max || self.depth == u16::MAX {
            let pos = (ctx.start().get_start(), ctx.stop().get_stop());
            return Err(ANTLRError::OtherError(Arc::new(ParseError {
                source: None,
                pos,
                msg: format!("Recursion limit of {} exceeded", self.max),
                expr_id: 0,
                source_info: None,
            })));
        }
        Ok(())
    }

    fn exit_every_rule(
        &mut self,
        _ctx: &<CELParserContextType as ParserNodeType>::Type,
    ) -> Result<(), ANTLRError> {
        Ok(())
    }
}

struct ParserErrorListener {
    parse_errors: Rc<RefCell<Vec<ParseError>>>,
}

impl<'a, T: Recognizer<'a>> ErrorListener<'a, T> for ParserErrorListener {
    fn syntax_error(
        &self,
        _recognizer: &T,
        _offending_symbol: Option<&<T::TF as TokenFactory<'a>>::Inner>,
        line: isize,
        column: isize,
        msg: &str,
        _error: Option<&ANTLRError>,
    ) {
        self.parse_errors.borrow_mut().push(ParseError {
            source: None,
            pos: (line, column + 1),
            msg: format!("Syntax error: {msg}"),
            expr_id: 0,
            source_info: None,
        })
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl ParseTreeVisitorCompat<'_> for Parser {
    type Node = r#gen::CELParserContextType;
    type Return = IdedExpr;
    fn temp_result(&mut self) -> &mut Self::Return {
        &mut self.ast.expr
    }

    fn visit(&mut self, node: &<Self::Node as ParserNodeType<'_>>::Type) -> Self::Return {
        // println!("{node:?}");
        self.visit_node(node);
        mem::take(self.temp_result())
    }

    fn aggregate_results(&self, _aggregate: Self::Return, next: Self::Return) -> Self::Return {
        next
    }
}

impl r#gen::CELVisitorCompat<'_> for Parser {
    fn visit_start(&mut self, ctx: &StartContext<'_>) -> Self::Return {
        match &ctx.expr() {
            None => self.report_error::<ParseError, _>(
                ctx.start().deref(),
                None,
                "No `ExprContextAll`!",
            ),
            Some(expr) => self.visit(expr.as_ref()),
        }
    }

    fn visit_expr(&mut self, ctx: &ExprContext<'_>) -> Self::Return {
        match &ctx.op {
            None => match &ctx.e {
                None => self.report_error::<ParseError, _>(
                    ctx.start().deref(),
                    None,
                    "No `ConditionalOrContextAll`!",
                ),
                Some(e) => <Self as ParseTreeVisitorCompat>::visit(self, e.as_ref()),
            },
            Some(op) => {
                if let (Some(e), Some(e1), Some(e2)) = (&ctx.e, &ctx.e1, &ctx.e2) {
                    let result = self.visit(e.as_ref());
                    let op_id = self.helper.next_id(op);
                    let if_true = self.visit(e1.as_ref());
                    let if_false = self.visit(e2.as_ref());
                    self.global_call_or_macro(
                        op_id,
                        operators::CONDITIONAL.to_string(),
                        vec![result, if_true, if_false],
                    )
                } else {
                    self.report_error::<ParseError, _>(
                        ctx.start().deref(),
                        None,
                        format!(
                            "Incomplete `ExprContext` for `{}` expression!",
                            operators::CONDITIONAL
                        ),
                    )
                }
            }
        }
    }

    fn visit_conditionalOr(&mut self, ctx: &ConditionalOrContext<'_>) -> Self::Return {
        let result = match &ctx.e {
            None => {
                self.report_error::<ParseError, _>(
                    ctx.start().deref(),
                    None,
                    "No `ConditionalAndContextAll`!",
                );
                IdedExpr::default()
            }
            Some(e) => <Self as ParseTreeVisitorCompat>::visit(self, e.as_ref()),
        };
        if ctx.ops.is_empty() {
            result
        } else {
            let mut l = self.new_logic_manager(operators::LOGICAL_OR, result);
            let rest = &ctx.e1;
            if ctx.ops.len() > rest.len() {
                // why is >= not ok?
                self.report_error::<ParseError, _>(
                    &ctx.start(),
                    None,
                    "unexpected character, wanted '||'",
                );
                return IdedExpr::default();
            }
            for (i, op) in ctx.ops.iter().enumerate() {
                let next = self.visit(rest[i].deref());
                let op_id = self.helper.next_id(op);
                l.add_term(op_id, next)
            }
            l.expr()
        }
    }

    fn visit_conditionalAnd(&mut self, ctx: &ConditionalAndContext<'_>) -> Self::Return {
        let result = match &ctx.e {
            None => self.report_error::<ParseError, _>(
                ctx.start().deref(),
                None,
                "No `RelationContextAll`!",
            ),
            Some(e) => <Self as ParseTreeVisitorCompat>::visit(self, e.as_ref()),
        };
        if ctx.ops.is_empty() {
            result
        } else {
            let mut l = self.new_logic_manager(operators::LOGICAL_AND, result);
            let rest = &ctx.e1;
            if ctx.ops.len() > rest.len() {
                // why is >= not ok?
                self.report_error::<ParseError, _>(
                    &ctx.start(),
                    None,
                    "unexpected character, wanted '&&'",
                );
                return IdedExpr::default();
            }
            for (i, op) in ctx.ops.iter().enumerate() {
                let next = self.visit(rest[i].deref());
                let op_id = self.helper.next_id(op);
                l.add_term(op_id, next)
            }
            l.expr()
        }
    }

    fn visit_relation(&mut self, ctx: &RelationContext<'_>) -> Self::Return {
        if ctx.op.is_none() {
            match ctx.calc() {
                None => self.report_error::<ParseError, _>(
                    ctx.start().deref(),
                    None,
                    "No `CalcContextAll`!",
                ),
                Some(calc) => <Self as ParseTreeVisitorCompat>::visit(self, calc.as_ref()),
            }
        } else {
            match &ctx.op {
                None => <Self as ParseTreeVisitorCompat>::visit_children(self, ctx),
                Some(op) => {
                    if let (Some(lhs), Some(rhs)) = (ctx.relation(0), ctx.relation(1)) {
                        let lhs = self.visit(lhs.as_ref());
                        let op_id = self.helper.next_id(op.as_ref());
                        let rhs = self.visit(rhs.as_ref());
                        match operators::find_operator(op.get_text()) {
                            None => {
                                self.report_error::<ParseError, _>(
                                    op.as_ref(),
                                    None,
                                    format!("Unknown `{}` operator!", op.get_text()),
                                );
                                IdedExpr::default()
                            }
                            Some(op) => {
                                self.global_call_or_macro(op_id, op.to_string(), vec![lhs, rhs])
                            }
                        }
                    } else {
                        self.report_error::<ParseError, _>(
                            ctx.start().deref(),
                            None,
                            format!("Incomplete `RelationContext` for `{:?}`!", ctx.op),
                        )
                    }
                }
            }
        }
    }

    fn visit_calc(&mut self, ctx: &CalcContext<'_>) -> Self::Return {
        match &ctx.op {
            None => match &ctx.unary() {
                None => self.report_error::<ParseError, _>(
                    ctx.start().deref(),
                    None,
                    "No `UnaryContextAll`!",
                ),
                Some(unary) => self.visit(unary.as_ref()),
            },
            Some(op) => {
                if let (Some(lhs), Some(rhs)) = (ctx.calc(0), ctx.calc(1)) {
                    let lhs = self.visit(lhs.as_ref());
                    let op_id = self.helper.next_id(op);
                    let rhs = self.visit(rhs.as_ref());
                    match operators::find_operator(op.get_text()) {
                        None => self.report_error::<ParseError, _>(
                            op,
                            None,
                            format!("Unknown `{}` operator!", op.get_text()),
                        ),
                        Some(op) => {
                            self.global_call_or_macro(op_id, op.to_string(), vec![lhs, rhs])
                        }
                    }
                } else {
                    self.report_error::<ParseError, _>(
                        ctx.start().deref(),
                        None,
                        "Incomplete `CalcContext`!",
                    )
                }
            }
        }
    }

    fn visit_MemberExpr(&mut self, ctx: &MemberExprContext<'_>) -> Self::Return {
        match &ctx.member() {
            None => {
                self.report_error::<ParseError, _>(&ctx.start(), None, "No `MemberContextAll`!")
            }
            Some(ctx) => <Self as ParseTreeVisitorCompat>::visit(self, ctx.as_ref()),
        }
    }

    fn visit_LogicalNot(&mut self, ctx: &LogicalNotContext<'_>) -> Self::Return {
        match &ctx.member() {
            None => {
                self.report_error::<ParseError, _>(&ctx.start(), None, "No `MemberContextAll`!");
                IdedExpr::default()
            }
            Some(member) => {
                if ctx.ops.len().is_multiple_of(2) {
                    self.visit(member.as_ref());
                }
                let op_id = self.helper.next_id(&ctx.ops[0]);
                let target = self.visit(member.as_ref());
                self.global_call_or_macro(op_id, operators::LOGICAL_NOT.to_string(), vec![target])
            }
        }
    }

    fn visit_Negate(&mut self, ctx: &NegateContext<'_>) -> Self::Return {
        match &ctx.member() {
            None => {
                self.report_error::<ParseError, _>(&ctx.start(), None, "No `MemberContextAll`!")
            }
            Some(member) => {
                if ctx.ops.len().is_multiple_of(2) {
                    self.visit(member.as_ref());
                }
                let op_id = self.helper.next_id(&ctx.ops[0]);
                let target = self.visit(member.as_ref());
                self.global_call_or_macro(op_id, operators::NEGATE.to_string(), vec![target])
            }
        }
    }

    fn visit_MemberCall(&mut self, ctx: &MemberCallContext<'_>) -> Self::Return {
        if let (Some(operand), Some(id), Some(open)) = (&ctx.member(), &ctx.id, &ctx.open) {
            let operand = self.visit(operand.as_ref());
            let id = id.get_text();
            let op_id = self.helper.next_id(open.as_ref());
            let args = ctx
                .args
                .iter()
                .flat_map(|arg| &arg.e)
                .map(|arg| self.visit(arg.deref()))
                .collect::<Vec<IdedExpr>>();
            self.receiver_call_or_macro(op_id, id.to_string(), operand, args)
        } else {
            self.report_error::<ParseError, _>(
                &ctx.start(),
                None,
                "Incomplete `MemberCallContext`!",
            )
        }
    }

    fn visit_Select(&mut self, ctx: &SelectContext<'_>) -> Self::Return {
        if let (Some(member), Some(id), Some(op)) = (&ctx.member(), &ctx.id, &ctx.op) {
            let operand = self.visit(member.as_ref());
            let field = id.get_text();
            if let Some(_opt) = &ctx.opt {
                return if self.enable_optional_syntax {
                    let field_literal = self
                        .helper
                        .next_expr(op.as_ref(), Expr::Literal(CelVal::String(field.clone())));
                    let op_id = self.helper.next_id(op.as_ref());
                    self.global_call_or_macro(
                        op_id,
                        operators::OPT_SELECT.to_string(),
                        vec![operand, field_literal],
                    )
                } else {
                    self.report_error::<ParseError, _>(op.as_ref(), None, "unsupported syntax '.?'")
                };
            }
            self.helper.next_expr(
                op.as_ref(),
                Expr::Select(SelectExpr {
                    operand: Box::new(operand),
                    field,
                    test: false,
                }),
            )
        } else {
            self.report_error::<ParseError, _>(&ctx.start(), None, "Incomplete `SelectContext`!")
        }
    }

    fn visit_PrimaryExpr(&mut self, ctx: &PrimaryExprContext<'_>) -> Self::Return {
        match &ctx.primary() {
            None => {
                self.report_error::<ParseError, _>(&ctx.start(), None, "No `PrimaryContextAll`!")
            }
            Some(primary) => <Self as ParseTreeVisitorCompat>::visit(self, primary.as_ref()),
        }
    }

    fn visit_Index(&mut self, ctx: &IndexContext<'_>) -> Self::Return {
        if let (Some(member), Some(index)) = (&ctx.member(), &ctx.index) {
            let target = self.visit(member.as_ref());
            match &ctx.op {
                None => self.report_error::<ParseError, _>(&ctx.start(), None, "No `Index`!"),
                Some(op) => {
                    let op_id = self.helper.next_id(op);
                    let index = self.visit(index.as_ref());
                    if let Some(_opt) = &ctx.opt {
                        return if self.enable_optional_syntax {
                            self.global_call_or_macro(
                                op_id,
                                operators::OPT_INDEX.to_string(),
                                vec![target, index],
                            )
                        } else {
                            self.report_error::<ParseError, _>(
                                op.as_ref(),
                                None,
                                "unsupported syntax '[?'",
                            )
                        };
                    }
                    self.global_call_or_macro(
                        op_id,
                        operators::INDEX.to_string(),
                        vec![target, index],
                    )
                }
            }
        } else {
            self.report_error::<ParseError, _>(&ctx.start(), None, "Incomplete `IndexContext`!")
        }
    }

    fn visit_Ident(&mut self, ctx: &IdentContext<'_>) -> Self::Return {
        match &ctx.id {
            None => {
                self.report_error::<ParseError, _>(&ctx.start(), None, "No `Identifier`!");
                IdedExpr::default()
            }
            Some(id) => {
                let ident = id.clone().text;
                self.helper
                    .next_expr(id.deref(), Expr::Ident(ident.to_string()))
            }
        }
    }

    fn visit_GlobalCall(&mut self, ctx: &GlobalCallContext<'_>) -> Self::Return {
        match &ctx.id {
            None => IdedExpr::default(),
            Some(id) => {
                let mut id = id.get_text().to_string();
                if ctx.leadingDot.is_some() {
                    id = format!(".{id}");
                }
                let op_id = self.helper.next_id_for_token(ctx.op.as_deref());
                let args = ctx
                    .args
                    .iter()
                    .flat_map(|arg| &arg.e)
                    .map(|arg| self.visit(arg.deref()))
                    .collect::<Vec<IdedExpr>>();
                self.global_call_or_macro(op_id, id, args)
            }
        }
    }

    fn visit_Nested(&mut self, ctx: &NestedContext<'_>) -> Self::Return {
        match &ctx.e {
            None => {
                self.report_error::<ParseError, _>(
                    ctx.start().deref(),
                    None,
                    "No `ExprContextAll`!",
                );
                IdedExpr::default()
            }
            Some(e) => self.visit(e.as_ref()),
        }
    }

    fn visit_CreateList(&mut self, ctx: &CreateListContext<'_>) -> Self::Return {
        let list_id = self.helper.next_id_for_token(ctx.op.as_deref());
        let (elements, optionals) = match &ctx.elems {
            None => (Vec::default(), Vec::default()),
            Some(elements) => self.list_initializer_list(elements.deref()),
        };
        IdedExpr {
            id: list_id,
            expr: Expr::List(ListExpr::new_with_optionals(elements, optionals)),
        }
    }

    fn visit_CreateStruct(&mut self, ctx: &CreateStructContext<'_>) -> Self::Return {
        let struct_id = self.helper.next_id_for_token(ctx.op.as_deref());
        let entries = match &ctx.entries {
            Some(entries) => self.map_initializer_list(entries.deref()),
            None => Vec::default(),
        };
        IdedExpr {
            id: struct_id,
            expr: Expr::Map(MapExpr { entries }),
        }
    }

    fn visit_CreateMessage(&mut self, ctx: &CreateMessageContext<'_>) -> Self::Return {
        let mut message_name = String::new();
        for id in &ctx.ids {
            if !message_name.is_empty() {
                message_name.push('.');
            }
            message_name.push_str(id.get_text());
        }
        if ctx.leadingDot.is_some() {
            message_name = format!(".{message_name}");
        }
        let op_id = match &ctx.op {
            None => {
                self.report_error::<ParseError, _>(&ctx.start(), None, "No `CommonToken`!");
                return IdedExpr::default();
            }
            Some(op) => self.helper.next_id(op.as_ref()),
        };
        let entries = match &ctx.entries {
            None => vec![],
            Some(entries) => self.field_initializer_list(entries),
        };
        IdedExpr {
            id: op_id,
            expr: Expr::Struct(StructExpr {
                type_name: message_name,
                entries,
            }),
        }
    }

    fn visit_ConstantLiteral(&mut self, ctx: &ConstantLiteralContext<'_>) -> Self::Return {
        if let Some(literal) = ctx.literal().as_deref() {
            <Self as ParseTreeVisitorCompat>::visit(self, literal)
        } else {
            self.report_error::<ParseError, _>(&ctx.start(), None, "Incomplete ConstantLiteral!")
        }
    }

    fn visit_Int(&mut self, ctx: &IntContext<'_>) -> Self::Return {
        let string = ctx.get_text();
        if let Some(token) = ctx.tok.as_ref() {
            let val = match if let Some(string) = string.strip_prefix("0x") {
                i64::from_str_radix(string, 16)
            } else {
                string.parse::<i64>()
            } {
                Ok(v) => v,
                Err(e) => return self.report_error(token, Some(e), "invalid int literal"),
            };
            self.helper
                .next_expr(token, Expr::Literal(CelVal::Int(val)))
        } else {
            self.report_error::<ParseError, _>(&ctx.start(), None, "Incomplete Int!")
        }
    }

    fn visit_Uint(&mut self, ctx: &UintContext<'_>) -> Self::Return {
        let mut string = ctx.get_text();
        string.truncate(string.len() - 1);
        if let Some(token) = ctx.tok.as_ref() {
            let val = match if let Some(string) = string.strip_prefix("0x") {
                u64::from_str_radix(string, 16)
            } else {
                string.parse::<u64>()
            } {
                Ok(v) => v,
                Err(e) => return self.report_error(token, Some(e), "invalid uint literal"),
            };
            self.helper
                .next_expr(token, Expr::Literal(CelVal::UInt(val)))
        } else {
            self.report_error::<ParseError, _>(&ctx.start(), None, "Incomplete Uint!")
        }
    }

    fn visit_Double(&mut self, ctx: &DoubleContext<'_>) -> Self::Return {
        let string = ctx.get_text();
        if let Some(token) = ctx.tok.as_ref() {
            match string.parse::<f64>() {
                Ok(d) if d.is_finite() => self
                    .helper
                    .next_expr(token, Expr::Literal(CelVal::Double(d))),
                Err(e) => self.report_error(token, Some(e), "invalid double literal"),
                _ => self.report_error(token, None::<ParseError>, "invalid double literal"),
            }
        } else {
            self.report_error::<ParseError, _>(
                &ctx.start(),
                None::<ParseError>,
                "Incomplete double!",
            )
        }
    }

    fn visit_String(&mut self, ctx: &StringContext<'_>) -> Self::Return {
        if let Some(token) = ctx.tok.as_deref() {
            match parse::parse_string(&ctx.get_text()) {
                Ok(string) => self
                    .helper
                    .next_expr(token, Expr::Literal(CelVal::String(string))),
                Err(e) => self.report_error::<ParseError, _>(
                    token,
                    None,
                    format!("invalid string literal: {e:?}"),
                ),
            }
        } else {
            self.report_error::<ParseError, _>(
                &ctx.start(),
                None::<ParseError>,
                "Incomplete string!",
            )
        }
    }

    fn visit_Bytes(&mut self, ctx: &BytesContext<'_>) -> Self::Return {
        if let Some(token) = ctx.tok.as_deref() {
            let string = ctx.get_text();
            match parse::parse_bytes(&string[2..string.len() - 1]) {
                Ok(bytes) => self
                    .helper
                    .next_expr(token, Expr::Literal(CelVal::Bytes(bytes))),
                Err(e) => {
                    self.report_error::<ParseError, _>(
                        token,
                        None,
                        format!("invalid bytes literal: {e:?}"),
                    );
                    IdedExpr::default()
                }
            }
        } else {
            self.report_error::<ParseError, _>(
                &ctx.start(),
                None::<ParseError>,
                "Incomplete bytes!",
            )
        }
    }

    fn visit_BoolTrue(&mut self, ctx: &BoolTrueContext<'_>) -> Self::Return {
        match ctx.tok.as_deref() {
            Some(tok) => self
                .helper
                .next_expr(tok, Expr::Literal(CelVal::Boolean(true))),
            None => self.report_error::<ParseError, _>(&ctx.start(), None, "Incomplete bool!"),
        }
    }

    fn visit_BoolFalse(&mut self, ctx: &BoolFalseContext<'_>) -> Self::Return {
        match ctx.tok.as_deref() {
            Some(token) => self
                .helper
                .next_expr(token, Expr::Literal(CelVal::Boolean(false))),
            None => self.report_error::<ParseError, _>(&ctx.start(), None, "Incomplete bool!"),
        }
    }

    fn visit_Null(&mut self, ctx: &NullContext<'_>) -> Self::Return {
        match ctx.tok.as_deref() {
            Some(token) => self.helper.next_expr(token, Expr::Literal(CelVal::Null)),
            None => self.report_error::<ParseError, _>(&ctx.start(), None, "Incomplete null!"),
        }
    }
}

pub struct ParserHelper {
    source_info: SourceInfo,
    next_id: u64,
}

impl Default for ParserHelper {
    fn default() -> Self {
        Self {
            source_info: SourceInfo::default(),
            next_id: 1,
        }
    }
}

impl ParserHelper {
    fn next_id(&mut self, token: &CommonToken) -> u64 {
        let id = self.next_id;
        self.source_info
            .add_offset(id, token.start as u32, token.stop as u32);
        self.next_id += 1;
        id
    }

    fn next_id_for_token(&mut self, token: Option<&CommonToken>) -> u64 {
        match token {
            None => 0,
            Some(token) => self.next_id(token),
        }
    }

    fn next_id_for(&mut self, id: u64) -> u64 {
        let (start, stop) = self.source_info.offset_for(id).expect("invalid offset");
        let id = self.next_id;
        self.source_info.add_offset(id, start, stop);
        self.next_id += 1;
        id
    }

    pub fn next_expr(&mut self, token: &CommonToken, expr: Expr) -> IdedExpr {
        IdedExpr {
            id: self.next_id(token),
            expr,
        }
    }

    pub fn next_expr_for(&mut self, id: u64, expr: Expr) -> IdedExpr {
        IdedExpr {
            id: self.next_id_for(id),
            expr,
        }
    }
}

struct LogicManager {
    function: String,
    terms: Vec<IdedExpr>,
    ops: Vec<u64>,
}

impl LogicManager {
    pub(crate) fn expr(mut self) -> IdedExpr {
        if self.terms.len() == 1 {
            self.terms.pop().expect("expected at least one term")
        } else {
            self.balanced_tree(0, self.ops.len() - 1)
        }
    }

    pub(crate) fn add_term(&mut self, op_id: u64, expr: IdedExpr) {
        self.terms.push(expr);
        self.ops.push(op_id);
    }

    fn balanced_tree(&mut self, lo: usize, hi: usize) -> IdedExpr {
        let mid = (lo + hi).div_ceil(2);

        let left = if mid == lo {
            mem::take(&mut self.terms[mid])
        } else {
            self.balanced_tree(lo, mid - 1)
        };

        let right = if mid == hi {
            mem::take(&mut self.terms[mid + 1])
        } else {
            self.balanced_tree(mid + 1, hi)
        };

        IdedExpr {
            id: self.ops[mid],
            expr: Expr::Call(CallExpr {
                target: None,
                func_name: self.function.clone(),
                args: vec![left, right],
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::iter;

    use super::*;
    use crate::IdedExpr;
    use crate::common::ast::{ComprehensionExpr, EntryExpr, Expr};

    #[derive(Default)]
    struct TestInfo {
        // I contains the input expression to be parsed.
        i: &'static str,

        // P contains the type/id adorned debug output of the expression tree.
        p: &'static str,

        // E contains the expected error output for a failed parse, or "" if the parse is expected to be successful.
        e: &'static str,
        // L contains the expected source adorned debug output of the expression tree.
        // l: String,

        // M contains the expected adorned debug output of the macro calls map
        // m: String,

        // Options to be configured with the parser before parsing the expression.
        enable_optional_syntax: bool,
    }

    #[test]
    fn test_bad_input() {
        let expressions = [
            "1 + ()", "/", ".", "@foo", "x(1,)", "\x0a", "\n", "", "!-\u{1}",
        ];
        for expr in expressions {
            assert!(
                Parser::new().parse(expr).is_err(),
                "Expression `{}` should not parse",
                expr
            );
        }
    }

    #[test]
    fn test_comments() {
        let expression = r#"
        // This is a comment
        this.is.not()

        // We don't care!

        "#;
        assert!(Parser::new().parse(expression).is_ok());
    }

    #[test]
    fn recursion_limits() {
        let expressions = [
            "[[[1]]]",
            "(((1)))",
            "{1: {2: {3: 'none'}}}",
            "type(type(type(1)))",
            "[{'a': size([])}]",
            "{}.map(a, a.map(b, b.map(c, c)))",
        ];
        for expr in expressions {
            assert!(
                Parser::new().max_recursion_depth(3).parse(expr).is_ok(),
                "Expression `{}` should parse",
                expr
            );
            assert!(
                Parser::new().max_recursion_depth(2).parse(expr).is_err(),
                "Expression `{}` should not parse",
                expr
            );
        }
        let expressions = [
            "[[[[[[[[[[[[1]]]]]]]]]]]]",
            "((((((((((((1))))))))))))",
            "{1: {2: {3: {4: {5: {6: {1: {2: {3: {4: {5: {6: 'none'}}}}}}}}}}}}",
            "type(type(type(type(type(type(type(type(type(type(type(type(1))))))))))))",
            "[{'a': size([{'1':size([{'1':size([[[[]]]])}])}])}]",
        ];
        for expr in expressions {
            assert!(
                Parser::new().max_recursion_depth(12).parse(expr).is_ok(),
                "Expression `{}` should parse",
                expr
            );
            assert!(
                Parser::new().max_recursion_depth(11).parse(expr).is_err(),
                "Expression `{}` should not parse",
                expr
            );
        }
        assert!(Parser::new().max_recursion_depth(0).parse("1 + 1").is_ok());
        assert!(
            Parser::new()
                .max_recursion_depth(0)
                .parse("(1 + 1)")
                .is_err()
        );
    }

    #[test]
    fn test() {
        let test_cases = [
            TestInfo {
                i: r#""A""#,
                p: r#""A"^#1:*expr.Constant_StringValue#"#,
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: r#"true"#,
                p: r#"true^#1:*expr.Constant_BoolValue#"#,
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: r#"false"#,
                p: r#"false^#1:*expr.Constant_BoolValue#"#,
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "0",
                p: "0^#1:*expr.Constant_Int64Value#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "42",
                p: "42^#1:*expr.Constant_Int64Value#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "0xF",
                p: "15^#1:*expr.Constant_Int64Value#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "0u",
                p: "0u^#1:*expr.Constant_Uint64Value#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "23u",
                p: "23u^#1:*expr.Constant_Uint64Value#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "24u",
                p: "24u^#1:*expr.Constant_Uint64Value#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "0xFu",
                p: "15u^#1:*expr.Constant_Uint64Value#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "-1",
                p: "-1^#1:*expr.Constant_Int64Value#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "4--4",
                p: r#"_-_(
    4^#1:*expr.Constant_Int64Value#,
    -4^#3:*expr.Constant_Int64Value#
)^#2:*expr.Expr_CallExpr#"#,
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "4--4.1",
                p: r#"_-_(
    4^#1:*expr.Constant_Int64Value#,
    -4.1^#3:*expr.Constant_DoubleValue#
)^#2:*expr.Expr_CallExpr#"#,
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: r#"b"abc""#,
                p: r#"b"abc"^#1:*expr.Constant_BytesValue#"#,
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "23.39",
                p: "23.39^#1:*expr.Constant_DoubleValue#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "!a",
                p: "!_(
    a^#2:*expr.Expr_IdentExpr#
)^#1:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "null",
                p: "null^#1:*expr.Constant_NullValue#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a",
                p: "a^#1:*expr.Expr_IdentExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a?b:c",
                p: "_?_:_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#,
    c^#4:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a || b",
                p: "_||_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#2:*expr.Expr_IdentExpr#
)^#3:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a || b || c || d || e || f ",
                p: "_||_(
    _||_(
        _||_(
            a^#1:*expr.Expr_IdentExpr#,
            b^#2:*expr.Expr_IdentExpr#
        )^#3:*expr.Expr_CallExpr#,
        c^#4:*expr.Expr_IdentExpr#
    )^#5:*expr.Expr_CallExpr#,
    _||_(
        _||_(
            d^#6:*expr.Expr_IdentExpr#,
            e^#8:*expr.Expr_IdentExpr#
        )^#9:*expr.Expr_CallExpr#,
        f^#10:*expr.Expr_IdentExpr#
    )^#11:*expr.Expr_CallExpr#
)^#7:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a && b",
                p: "_&&_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#2:*expr.Expr_IdentExpr#
)^#3:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a && b && c && d && e && f && g",
                p: "_&&_(
    _&&_(
        _&&_(
            a^#1:*expr.Expr_IdentExpr#,
            b^#2:*expr.Expr_IdentExpr#
        )^#3:*expr.Expr_CallExpr#,
        _&&_(
            c^#4:*expr.Expr_IdentExpr#,
            d^#6:*expr.Expr_IdentExpr#
        )^#7:*expr.Expr_CallExpr#
    )^#5:*expr.Expr_CallExpr#,
    _&&_(
        _&&_(
            e^#8:*expr.Expr_IdentExpr#,
            f^#10:*expr.Expr_IdentExpr#
        )^#11:*expr.Expr_CallExpr#,
        g^#12:*expr.Expr_IdentExpr#
    )^#13:*expr.Expr_CallExpr#
)^#9:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a && b && c && d || e && f && g && h",
                p: "_||_(
    _&&_(
        _&&_(
            a^#1:*expr.Expr_IdentExpr#,
            b^#2:*expr.Expr_IdentExpr#
        )^#3:*expr.Expr_CallExpr#,
        _&&_(
            c^#4:*expr.Expr_IdentExpr#,
            d^#6:*expr.Expr_IdentExpr#
        )^#7:*expr.Expr_CallExpr#
    )^#5:*expr.Expr_CallExpr#,
    _&&_(
        _&&_(
            e^#8:*expr.Expr_IdentExpr#,
            f^#9:*expr.Expr_IdentExpr#
        )^#10:*expr.Expr_CallExpr#,
        _&&_(
            g^#11:*expr.Expr_IdentExpr#,
            h^#13:*expr.Expr_IdentExpr#
        )^#14:*expr.Expr_CallExpr#
    )^#12:*expr.Expr_CallExpr#
)^#15:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a + b",
                p: "_+_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a - b",
                p: "_-_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a * b",
                p: "_*_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a / b",
                p: "_/_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a % b",
                p: "_%_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a in b",
                p: "@in(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a == b",
                p: "_==_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a != b",
                p: "_!=_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a > b",
                p: "_>_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a >= b",
                p: "_>=_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a < b",
                p: "_<_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a <= b",
                p: "_<=_(
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a.b",
                p: "a^#1:*expr.Expr_IdentExpr#.b^#2:*expr.Expr_SelectExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a.b.c",
                p: "a^#1:*expr.Expr_IdentExpr#.b^#2:*expr.Expr_SelectExpr#.c^#3:*expr.Expr_SelectExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a[b]",
                p: "_[_](
    a^#1:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "(a)",
                p: "a^#1:*expr.Expr_IdentExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "((a))",
                p: "a^#1:*expr.Expr_IdentExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a()",
                p: "a()^#1:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a(b)",
                p: "a(
    b^#2:*expr.Expr_IdentExpr#
)^#1:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a(b, c)",
                p: "a(
    b^#2:*expr.Expr_IdentExpr#,
    c^#3:*expr.Expr_IdentExpr#
)^#1:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a.b()",
                p: "a^#1:*expr.Expr_IdentExpr#.b()^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "a.b(c)",
                p: "a^#1:*expr.Expr_IdentExpr#.b(
    c^#3:*expr.Expr_IdentExpr#
)^#2:*expr.Expr_CallExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "foo{ }",
                p: "foo{}^#1:*expr.Expr_StructExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "foo{ a:b }",
                p: "foo{
    a:b^#3:*expr.Expr_IdentExpr#^#2:*expr.Expr_CreateStruct_Entry#
}^#1:*expr.Expr_StructExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "foo{ a:b, c:d }",
                p: "foo{
    a:b^#3:*expr.Expr_IdentExpr#^#2:*expr.Expr_CreateStruct_Entry#,
    c:d^#5:*expr.Expr_IdentExpr#^#4:*expr.Expr_CreateStruct_Entry#
}^#1:*expr.Expr_StructExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "{}",
                p: "{}^#1:*expr.Expr_StructExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "{a: b, c: d}",
                p: "{
    a^#3:*expr.Expr_IdentExpr#:b^#4:*expr.Expr_IdentExpr#^#2:*expr.Expr_CreateStruct_Entry#,
    c^#6:*expr.Expr_IdentExpr#:d^#7:*expr.Expr_IdentExpr#^#5:*expr.Expr_CreateStruct_Entry#
}^#1:*expr.Expr_StructExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "[]",
                p: "[]^#1:*expr.Expr_ListExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "[a]",
                p: "[
    a^#2:*expr.Expr_IdentExpr#
]^#1:*expr.Expr_ListExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "[a, b, c]",
                p: "[
    a^#2:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#,
    c^#4:*expr.Expr_IdentExpr#
]^#1:*expr.Expr_ListExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "has(m.f)",
                p: "m^#2:*expr.Expr_IdentExpr#.f~test-only~^#4:*expr.Expr_SelectExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "m.exists(v, f)",
                p: "__comprehension__(
// Variable
v,
// Target
m^#1:*expr.Expr_IdentExpr#,
// Accumulator
@result,
// Init
false^#5:*expr.Constant_BoolValue#,
// LoopCondition
@not_strictly_false(
    !_(
        @result^#6:*expr.Expr_IdentExpr#
    )^#7:*expr.Expr_CallExpr#
)^#8:*expr.Expr_CallExpr#,
// LoopStep
_||_(
    @result^#9:*expr.Expr_IdentExpr#,
    f^#4:*expr.Expr_IdentExpr#
)^#10:*expr.Expr_CallExpr#,
// Result
@result^#11:*expr.Expr_IdentExpr#)^#12:*expr.Expr_ComprehensionExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "m.all(v, f)",
                p: "__comprehension__(
// Variable
v,
// Target
m^#1:*expr.Expr_IdentExpr#,
// Accumulator
@result,
// Init
true^#5:*expr.Constant_BoolValue#,
// LoopCondition
@not_strictly_false(
    @result^#6:*expr.Expr_IdentExpr#
)^#7:*expr.Expr_CallExpr#,
// LoopStep
_&&_(
    @result^#8:*expr.Expr_IdentExpr#,
    f^#4:*expr.Expr_IdentExpr#
)^#9:*expr.Expr_CallExpr#,
// Result
@result^#10:*expr.Expr_IdentExpr#)^#11:*expr.Expr_ComprehensionExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "m.existsOne(v, f)",
                p: "__comprehension__(
// Variable
v,
// Target
m^#1:*expr.Expr_IdentExpr#,
// Accumulator
@result,
// Init
0^#5:*expr.Constant_Int64Value#,
// LoopCondition
true^#6:*expr.Constant_BoolValue#,
// LoopStep
_?_:_(
    f^#4:*expr.Expr_IdentExpr#,
    _+_(
        @result^#7:*expr.Expr_IdentExpr#,
        1^#8:*expr.Constant_Int64Value#
    )^#9:*expr.Expr_CallExpr#,
    @result^#10:*expr.Expr_IdentExpr#
)^#11:*expr.Expr_CallExpr#,
// Result
_==_(
    @result^#12:*expr.Expr_IdentExpr#,
    1^#13:*expr.Constant_Int64Value#
)^#14:*expr.Expr_CallExpr#)^#15:*expr.Expr_ComprehensionExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "m.map(v, f)",
                p: "__comprehension__(
// Variable
v,
// Target
m^#1:*expr.Expr_IdentExpr#,
// Accumulator
@result,
// Init
[]^#5:*expr.Expr_ListExpr#,
// LoopCondition
true^#6:*expr.Constant_BoolValue#,
// LoopStep
_+_(
    @result^#7:*expr.Expr_IdentExpr#,
    [
        f^#4:*expr.Expr_IdentExpr#
    ]^#8:*expr.Expr_ListExpr#
)^#9:*expr.Expr_CallExpr#,
// Result
@result^#10:*expr.Expr_IdentExpr#)^#11:*expr.Expr_ComprehensionExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "m.map(v, p, f)",
                p: "__comprehension__(
// Variable
v,
// Target
m^#1:*expr.Expr_IdentExpr#,
// Accumulator
@result,
// Init
[]^#6:*expr.Expr_ListExpr#,
// LoopCondition
true^#7:*expr.Constant_BoolValue#,
// LoopStep
_?_:_(
    p^#4:*expr.Expr_IdentExpr#,
    _+_(
        @result^#8:*expr.Expr_IdentExpr#,
        [
            f^#5:*expr.Expr_IdentExpr#
        ]^#9:*expr.Expr_ListExpr#
    )^#10:*expr.Expr_CallExpr#,
    @result^#11:*expr.Expr_IdentExpr#
)^#12:*expr.Expr_CallExpr#,
// Result
@result^#13:*expr.Expr_IdentExpr#)^#14:*expr.Expr_ComprehensionExpr#",
                e: "",
                ..Default::default()
            },
            TestInfo {
                i: "m.filter(v, p)",
                p: "__comprehension__(
// Variable
v,
// Target
m^#1:*expr.Expr_IdentExpr#,
// Accumulator
@result,
// Init
[]^#5:*expr.Expr_ListExpr#,
// LoopCondition
true^#6:*expr.Constant_BoolValue#,
// LoopStep
_?_:_(
    p^#4:*expr.Expr_IdentExpr#,
    _+_(
        @result^#7:*expr.Expr_IdentExpr#,
        [
            v^#3:*expr.Expr_IdentExpr#
        ]^#8:*expr.Expr_ListExpr#
    )^#9:*expr.Expr_CallExpr#,
    @result^#10:*expr.Expr_IdentExpr#
)^#11:*expr.Expr_CallExpr#,
// Result
@result^#12:*expr.Expr_IdentExpr#)^#13:*expr.Expr_ComprehensionExpr#",
                e: "",
                ..Default::default()
            },
            // Parse error tests
            TestInfo {
                i: "0xFFFFFFFFFFFFFFFFF",
                p: "",
                e: "ERROR: <input>:1:1: invalid int literal
| 0xFFFFFFFFFFFFFFFFF
| ^",
                ..Default::default()
            },
            TestInfo {
                i: "0xFFFFFFFFFFFFFFFFFu",
                p: "",
                e: "ERROR: <input>:1:1: invalid uint literal
| 0xFFFFFFFFFFFFFFFFFu
| ^",
                ..Default::default()
            },
            TestInfo {
                i: "1.99e90000009",
                p: "",
                e: "ERROR: <input>:1:1: invalid double literal
| 1.99e90000009
| ^",
                ..Default::default()
            },
            TestInfo {
                i: "{",
                p: "",
                e: "ERROR: <input>:1:2: Syntax error: mismatched input '<EOF>' expecting {'[', '{', '}', '(', '.', ',', '-', '!', '?', 'true', 'false', 'null', NUM_FLOAT, NUM_INT, NUM_UINT, STRING, BYTES, IDENTIFIER}
| {
| .^",
                ..Default::default()
            },
            TestInfo {
                i: "*@a | b",
                p: "",
                e: "ERROR: <input>:1:1: Syntax error: extraneous input '*' expecting {'[', '{', '(', '.', '-', '!', 'true', 'false', 'null', NUM_FLOAT, NUM_INT, NUM_UINT, STRING, BYTES, IDENTIFIER}
| *@a | b
| ^
ERROR: <input>:1:2: Syntax error: token recognition error at: '@'
| *@a | b
| .^
ERROR: <input>:1:5: Syntax error: token recognition error at: '| '
| *@a | b
| ....^
ERROR: <input>:1:7: Syntax error: extraneous input 'b' expecting <EOF>
| *@a | b
| ......^",
                ..Default::default()
            },
            TestInfo {
                i: "a | b",
                p: "",
                e: "ERROR: <input>:1:3: Syntax error: token recognition error at: '| '
| a | b
| ..^
ERROR: <input>:1:5: Syntax error: extraneous input 'b' expecting <EOF>
| a | b
| ....^",
                ..Default::default()
            },
            TestInfo {
                i: "a.?b && a[?b]",
                p: "",
                e: "ERROR: <input>:1:2: unsupported syntax '.?'
| a.?b && a[?b]
| .^
ERROR: <input>:1:10: unsupported syntax '[?'
| a.?b && a[?b]
| .........^",
                enable_optional_syntax: false,
            },
            TestInfo {
            i: "a.?b[?0] && a[?c]",
            p: r#"_&&_(
    _[?_](
        _?._(
            a^#1:*expr.Expr_IdentExpr#,
            "b"^#2:*expr.Constant_StringValue#
        )^#3:*expr.Expr_CallExpr#,
        0^#5:*expr.Constant_Int64Value#
    )^#4:*expr.Expr_CallExpr#,
    _[?_](
        a^#6:*expr.Expr_IdentExpr#,
        c^#8:*expr.Expr_IdentExpr#
    )^#7:*expr.Expr_CallExpr#
)^#9:*expr.Expr_CallExpr#"#,
            e: "",
            enable_optional_syntax: true,
        },
        TestInfo {
            i: "{?'key': value}",
            p: r#"{
    ?"key"^#3:*expr.Constant_StringValue#:value^#4:*expr.Expr_IdentExpr#^#2:*expr.Expr_CreateStruct_Entry#
}^#1:*expr.Expr_StructExpr#"#,
            e: "",
            enable_optional_syntax: true,
        },
        TestInfo {
            i: "[?a, ?b]",
            p: r#"[
    a^#2:*expr.Expr_IdentExpr#,
    b^#3:*expr.Expr_IdentExpr#
]^#1:*expr.Expr_ListExpr#"#,
            e: "",
            enable_optional_syntax: true,
        },
        TestInfo {
            i: "[?a[?b]]",
            p: r#"[
    _[?_](
        a^#2:*expr.Expr_IdentExpr#,
        b^#4:*expr.Expr_IdentExpr#
    )^#3:*expr.Expr_CallExpr#
]^#1:*expr.Expr_ListExpr#"#,
            e: "",
            enable_optional_syntax: true,
        },
            TestInfo {
                i: "[?a, ?b]",
                p: "",
                e: "ERROR: <input>:1:2: unsupported syntax '?'
| [?a, ?b]
| .^
ERROR: <input>:1:6: unsupported syntax '?'
| [?a, ?b]
| .....^",
                enable_optional_syntax: false,
            },
            TestInfo {
                i: "Msg{?field: value}",
                p: r#"Msg{
    ?field:value^#3:*expr.Expr_IdentExpr#^#2:*expr.Expr_CreateStruct_Entry#
}^#1:*expr.Expr_StructExpr#"#,
                e: "",
                enable_optional_syntax: true,
            },
            TestInfo {
                i: "Msg{?field: value} && {?'key': value}",
                p: "",
                e: "ERROR: <input>:1:5: unsupported syntax '?'
| Msg{?field: value} && {?'key': value}
| ....^
ERROR: <input>:1:24: unsupported syntax '?'
| Msg{?field: value} && {?'key': value}
| .......................^",
                enable_optional_syntax: false,
            },
            TestInfo {
                i: "has(m)",
                p: "",
                e: "ERROR: <input>:1:5: invalid argument to has() macro
| has(m)
| ....^",
                ..Default::default()
            },
            TestInfo {
                i: "1.all(2, 3)",
                p: "",
                e: "ERROR: <input>:1:7: argument must be a simple name
| 1.all(2, 3)
| ......^",
                ..Default::default()
            },
        ];

        for test_case in test_cases {
            let parser = Parser::new().enable_optional_syntax(test_case.enable_optional_syntax);
            let result = parser.parse(test_case.i);
            if !test_case.p.is_empty() {
                assert_eq!(
                    to_go_like_string(result.as_ref().expect("Expected an AST")),
                    test_case.p,
                    "Expr `{}` failed",
                    test_case.i
                );
            }

            if !test_case.e.is_empty() {
                assert_eq!(
                    format!("{}", result.as_ref().expect_err("Expected an Err!")),
                    test_case.e,
                    "Error on `{}` failed",
                    test_case.i
                )
            }
        }
    }

    fn to_go_like_string(expr: &IdedExpr) -> String {
        let mut writer = DebugWriter::default();
        writer.buffer(expr);
        writer.done()
    }

    struct DebugWriter {
        buffer: String,
        indents: usize,
        line_start: bool,
    }

    impl Default for DebugWriter {
        fn default() -> Self {
            Self {
                buffer: String::default(),
                indents: 0,
                line_start: true,
            }
        }
    }

    impl DebugWriter {
        fn buffer(&mut self, expr: &IdedExpr) -> &Self {
            let e = match &expr.expr {
                Expr::Optimized { .. } => "optimized",
                Expr::Unspecified => "UNSPECIFIED!",
                Expr::Call(call) => {
                    if let Some(target) = &call.target {
                        self.buffer(target);
                        self.push(".");
                    }
                    self.push(call.func_name.as_str());
                    self.push("(");
                    if !call.args.is_empty() {
                        self.inc_indent();
                        self.newline();
                        for i in 0..call.args.len() {
                            if i > 0 {
                                self.push(",");
                                self.newline();
                            }
                            self.buffer(&call.args[i]);
                        }
                        self.dec_indent();
                        self.newline();
                    }
                    self.push(")");
                    &format!("^#{}:{}#", expr.id, "*expr.Expr_CallExpr")
                }
                Expr::Comprehension(comprehension) => {
                    self.push("__comprehension__(\n");
                    self.push_comprehension(comprehension);
                    &format!(")^#{}:{}#", expr.id, "*expr.Expr_ComprehensionExpr")
                }
                Expr::Ident(id) => &format!("{}^#{}:{}#", id, expr.id, "*expr.Expr_IdentExpr"),
                Expr::List(list) => {
                    self.push("[");
                    if !list.elements.is_empty() {
                        self.inc_indent();
                        self.newline();
                        for (i, element) in list.elements.iter().enumerate() {
                            if i > 0 {
                                self.push(",");
                                self.newline();
                            }
                            self.buffer(element);
                        }
                        self.dec_indent();
                        self.newline();
                    }
                    self.push("]");
                    &format!("^#{}:{}#", expr.id, "*expr.Expr_ListExpr")
                }
                Expr::Inline(_val) => todo!(),
                Expr::Literal(val) => match val {
                    CelVal::String(s) => {
                        &format!("\"{s}\"^#{}:{}#", expr.id, "*expr.Constant_StringValue")
                    }
                    CelVal::Boolean(b) => {
                        &format!("{b}^#{}:{}#", expr.id, "*expr.Constant_BoolValue")
                    }
                    CelVal::Int(i) => &format!("{i}^#{}:{}#", expr.id, "*expr.Constant_Int64Value"),
                    CelVal::UInt(u) => {
                        &format!("{u}u^#{}:{}#", expr.id, "*expr.Constant_Uint64Value")
                    }
                    CelVal::Double(f) => {
                        &format!("{f}^#{}:{}#", expr.id, "*expr.Constant_DoubleValue")
                    }
                    CelVal::Bytes(bytes) => &format!(
                        "b\"{}\"^#{}:{}#",
                        String::from_utf8_lossy(bytes),
                        expr.id,
                        "*expr.Constant_BytesValue"
                    ),
                    CelVal::Null => &format!("null^#{}:{}#", expr.id, "*expr.Constant_NullValue"),
                    t => &format!("WUT? {t:?}"),
                },
                Expr::Map(map) => {
                    self.push("{");
                    self.inc_indent();
                    if !map.entries.is_empty() {
                        self.newline();
                    }
                    for (i, entry) in map.entries.iter().enumerate() {
                        match &entry.expr {
                            EntryExpr::StructField(_) => panic!("WAT?!"),
                            EntryExpr::MapEntry(e) => {
                                if e.optional {
                                    self.push("?");
                                }
                                self.buffer(&e.key);
                                self.push(":");
                                self.buffer(&e.value);
                                self.push(&format!(
                                    "^#{}:{}#",
                                    entry.id, "*expr.Expr_CreateStruct_Entry"
                                ));
                            }
                        }
                        if i < map.entries.len() - 1 {
                            self.push(",");
                        }
                        self.newline();
                    }
                    self.dec_indent();
                    self.push("}");
                    &format!("^#{}:{}#", expr.id, "*expr.Expr_StructExpr")
                }
                Expr::Select(select) => {
                    self.buffer(select.operand.deref());
                    let suffix = if select.test { "~test-only~" } else { "" };
                    &format!(
                        ".{}{}^#{}:{}#",
                        select.field, suffix, expr.id, "*expr.Expr_SelectExpr"
                    )
                }
                Expr::Struct(s) => {
                    self.push(&s.type_name);
                    self.push("{");
                    self.inc_indent();
                    if !s.entries.is_empty() {
                        self.newline();
                    }
                    for (i, entry) in s.entries.iter().enumerate() {
                        match &entry.expr {
                            EntryExpr::StructField(field) => {
                                if field.optional {
                                    self.push("?");
                                }
                                self.push(&field.field);
                                self.push(":");
                                self.buffer(&field.value);
                                self.push(&format!(
                                    "^#{}:{}#",
                                    entry.id, "*expr.Expr_CreateStruct_Entry"
                                ));
                            }
                            EntryExpr::MapEntry(_) => panic!("WAT?!"),
                        }
                        if i < s.entries.len() - 1 {
                            self.push(",");
                        }
                        self.newline();
                    }
                    self.dec_indent();
                    self.push("}");
                    &format!("^#{}:{}#", expr.id, "*expr.Expr_StructExpr")
                }
            };
            self.push(e);
            self
        }

        fn push(&mut self, literal: &str) {
            self.indent();
            self.buffer.push_str(literal);
        }

        fn indent(&mut self) {
            if self.line_start {
                self.line_start = false;
                self.buffer.push_str(
                    iter::repeat_n("    ", self.indents)
                        .collect::<String>()
                        .as_str(),
                )
            }
        }

        fn newline(&mut self) {
            self.buffer.push('\n');
            self.line_start = true;
        }

        fn inc_indent(&mut self) {
            self.indents += 1;
        }

        fn dec_indent(&mut self) {
            self.indents -= 1;
        }

        fn done(self) -> String {
            self.buffer
        }

        fn push_comprehension(&mut self, comprehension: &ComprehensionExpr) {
            self.push("// Variable\n");
            self.push(comprehension.iter_var.as_str());
            self.push(",\n");
            self.push("// Target\n");
            self.buffer(&comprehension.iter_range);
            self.push(",\n");
            self.push("// Accumulator\n");
            self.push(comprehension.accu_var.as_str());
            self.push(",\n");
            self.push("// Init\n");
            self.buffer(&comprehension.accu_init);
            self.push(",\n");
            self.push("// LoopCondition\n");
            self.buffer(&comprehension.loop_cond);
            self.push(",\n");
            self.push("// LoopStep\n");
            self.buffer(&comprehension.loop_step);
            self.push(",\n");
            self.push("// Result\n");
            self.buffer(&comprehension.result);
        }
    }
}
