// Generated from CEL.g4 by ANTLR 4.13.2
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(nonstandard_style)]
#![allow(unused_imports)]
#![allow(unused_mut)]
#![allow(unused_braces)]
use std::any::{Any, TypeId};
use std::borrow::{Borrow, BorrowMut};
use std::cell::RefCell;
use std::convert::TryFrom;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::Arc;

use antlr4rust::atn::{ATN, INVALID_ALT};
use antlr4rust::atn_deserializer::ATNDeserializer;
use antlr4rust::dfa::DFA;
use antlr4rust::error_strategy::{DefaultErrorStrategy, ErrorStrategy};
use antlr4rust::errors::*;
use antlr4rust::int_stream::EOF;
use antlr4rust::parser::{BaseParser, Parser, ParserNodeType, ParserRecog};
use antlr4rust::parser_atn_simulator::ParserATNSimulator;
use antlr4rust::parser_rule_context::{BaseParserRuleContext, ParserRuleContext, cast, cast_mut};
use antlr4rust::recognizer::{Actions, Recognizer};
use antlr4rust::rule_context::{BaseRuleContext, CustomRuleContext, RuleContext};
use antlr4rust::token::{OwningToken, TOKEN_EOF, Token};
use antlr4rust::token_factory::{CommonTokenFactory, TokenAware, TokenFactory};
use antlr4rust::token_stream::TokenStream;
use antlr4rust::tree::*;
use antlr4rust::vocabulary::{Vocabulary, VocabularyImpl};
use antlr4rust::{PredictionContextCache, TidAble, TidExt, TokenSource, lazy_static};

use super::cellistener::*;
use super::celvisitor::*;

pub const CEL_EQUALS: i32 = 1;
pub const CEL_NOT_EQUALS: i32 = 2;
pub const CEL_IN: i32 = 3;
pub const CEL_LESS: i32 = 4;
pub const CEL_LESS_EQUALS: i32 = 5;
pub const CEL_GREATER_EQUALS: i32 = 6;
pub const CEL_GREATER: i32 = 7;
pub const CEL_LOGICAL_AND: i32 = 8;
pub const CEL_LOGICAL_OR: i32 = 9;
pub const CEL_LBRACKET: i32 = 10;
pub const CEL_RPRACKET: i32 = 11;
pub const CEL_LBRACE: i32 = 12;
pub const CEL_RBRACE: i32 = 13;
pub const CEL_LPAREN: i32 = 14;
pub const CEL_RPAREN: i32 = 15;
pub const CEL_DOT: i32 = 16;
pub const CEL_COMMA: i32 = 17;
pub const CEL_MINUS: i32 = 18;
pub const CEL_EXCLAM: i32 = 19;
pub const CEL_QUESTIONMARK: i32 = 20;
pub const CEL_COLON: i32 = 21;
pub const CEL_PLUS: i32 = 22;
pub const CEL_STAR: i32 = 23;
pub const CEL_SLASH: i32 = 24;
pub const CEL_PERCENT: i32 = 25;
pub const CEL_CEL_TRUE: i32 = 26;
pub const CEL_CEL_FALSE: i32 = 27;
pub const CEL_NUL: i32 = 28;
pub const CEL_WHITESPACE: i32 = 29;
pub const CEL_COMMENT: i32 = 30;
pub const CEL_NUM_FLOAT: i32 = 31;
pub const CEL_NUM_INT: i32 = 32;
pub const CEL_NUM_UINT: i32 = 33;
pub const CEL_STRING: i32 = 34;
pub const CEL_BYTES: i32 = 35;
pub const CEL_IDENTIFIER: i32 = 36;
pub const CEL_ESC_IDENTIFIER: i32 = 37;
pub const CEL_EOF: i32 = EOF;
pub const RULE_start: usize = 0;
pub const RULE_expr: usize = 1;
pub const RULE_conditionalOr: usize = 2;
pub const RULE_conditionalAnd: usize = 3;
pub const RULE_relation: usize = 4;
pub const RULE_calc: usize = 5;
pub const RULE_unary: usize = 6;
pub const RULE_member: usize = 7;
pub const RULE_primary: usize = 8;
pub const RULE_exprList: usize = 9;
pub const RULE_listInit: usize = 10;
pub const RULE_field_initializer_list: usize = 11;
pub const RULE_optField: usize = 12;
pub const RULE_mapInitializerList: usize = 13;
pub const RULE_escapeIdent: usize = 14;
pub const RULE_optExpr: usize = 15;
pub const RULE_literal: usize = 16;
pub const ruleNames: [&'static str; 17] = [
    "start",
    "expr",
    "conditionalOr",
    "conditionalAnd",
    "relation",
    "calc",
    "unary",
    "member",
    "primary",
    "exprList",
    "listInit",
    "field_initializer_list",
    "optField",
    "mapInitializerList",
    "escapeIdent",
    "optExpr",
    "literal",
];

pub const _LITERAL_NAMES: [Option<&'static str>; 29] = [
    None,
    Some("'=='"),
    Some("'!='"),
    Some("'in'"),
    Some("'<'"),
    Some("'<='"),
    Some("'>='"),
    Some("'>'"),
    Some("'&&'"),
    Some("'||'"),
    Some("'['"),
    Some("']'"),
    Some("'{'"),
    Some("'}'"),
    Some("'('"),
    Some("')'"),
    Some("'.'"),
    Some("','"),
    Some("'-'"),
    Some("'!'"),
    Some("'?'"),
    Some("':'"),
    Some("'+'"),
    Some("'*'"),
    Some("'/'"),
    Some("'%'"),
    Some("'true'"),
    Some("'false'"),
    Some("'null'"),
];
pub const _SYMBOLIC_NAMES: [Option<&'static str>; 38] = [
    None,
    Some("EQUALS"),
    Some("NOT_EQUALS"),
    Some("IN"),
    Some("LESS"),
    Some("LESS_EQUALS"),
    Some("GREATER_EQUALS"),
    Some("GREATER"),
    Some("LOGICAL_AND"),
    Some("LOGICAL_OR"),
    Some("LBRACKET"),
    Some("RPRACKET"),
    Some("LBRACE"),
    Some("RBRACE"),
    Some("LPAREN"),
    Some("RPAREN"),
    Some("DOT"),
    Some("COMMA"),
    Some("MINUS"),
    Some("EXCLAM"),
    Some("QUESTIONMARK"),
    Some("COLON"),
    Some("PLUS"),
    Some("STAR"),
    Some("SLASH"),
    Some("PERCENT"),
    Some("CEL_TRUE"),
    Some("CEL_FALSE"),
    Some("NUL"),
    Some("WHITESPACE"),
    Some("COMMENT"),
    Some("NUM_FLOAT"),
    Some("NUM_INT"),
    Some("NUM_UINT"),
    Some("STRING"),
    Some("BYTES"),
    Some("IDENTIFIER"),
    Some("ESC_IDENTIFIER"),
];
lazy_static! {
    static ref _shared_context_cache: Arc<PredictionContextCache> =
        Arc::new(PredictionContextCache::new());
    static ref VOCABULARY: Box<dyn Vocabulary> = Box::new(VocabularyImpl::new(
        _LITERAL_NAMES.iter(),
        _SYMBOLIC_NAMES.iter(),
        None
    ));
}

type BaseParserType<'input, I> = BaseParser<
    'input,
    CELParserExt<'input>,
    I,
    CELParserContextType,
    dyn CELListener<'input> + 'input,
>;

type TokenType<'input> = <LocalTokenFactory<'input> as TokenFactory<'input>>::Tok;
pub type LocalTokenFactory<'input> = CommonTokenFactory;

pub type CELTreeWalker<'input, 'a> =
    ParseTreeWalker<'input, 'a, CELParserContextType, dyn CELListener<'input> + 'a>;

/// Parser for CEL grammar
pub struct CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    base: BaseParserType<'input, I>,
    interpreter: Arc<ParserATNSimulator>,
    _shared_context_cache: Box<PredictionContextCache>,
    pub err_handler: Box<dyn ErrorStrategy<'input, BaseParserType<'input, I>>>,
}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn set_error_strategy(
        &mut self,
        strategy: Box<dyn ErrorStrategy<'input, BaseParserType<'input, I>>>,
    ) {
        self.err_handler = strategy
    }

    pub fn with_strategy(
        input: I,
        strategy: Box<dyn ErrorStrategy<'input, BaseParserType<'input, I>>>,
    ) -> Self {
        antlr4rust::recognizer::check_version("0", "5");
        let interpreter = Arc::new(ParserATNSimulator::new(
            _ATN.clone(),
            _decision_to_DFA.clone(),
            _shared_context_cache.clone(),
        ));
        Self {
            base: BaseParser::new_base_parser(
                input,
                Arc::clone(&interpreter),
                CELParserExt {
                    _pd: Default::default(),
                },
            ),
            interpreter,
            _shared_context_cache: Box::new(PredictionContextCache::new()),
            err_handler: strategy,
        }
    }
}

type DynStrategy<'input, I> = Box<dyn ErrorStrategy<'input, BaseParserType<'input, I>> + 'input>;

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn with_dyn_strategy(input: I) -> Self {
        Self::with_strategy(input, Box::new(DefaultErrorStrategy::new()))
    }
}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn new(input: I) -> Self {
        Self::with_strategy(input, Box::new(DefaultErrorStrategy::new()))
    }
}

/// Trait for monomorphized trait object that corresponds to the nodes of parse tree generated for CELParser
pub trait CELParserContext<'input>:
    for<'x> Listenable<dyn CELListener<'input> + 'x>
    + for<'x> Visitable<dyn CELVisitor<'input> + 'x>
    + ParserRuleContext<'input, TF = LocalTokenFactory<'input>, Ctx = CELParserContextType>
{
}

antlr4rust::coerce_from! { 'input : CELParserContext<'input> }

impl<'input, 'x, T> VisitableDyn<T> for dyn CELParserContext<'input> + 'input
where
    T: CELVisitor<'input> + 'x,
{
    fn accept_dyn(&self, visitor: &mut T) {
        self.accept(visitor as &mut (dyn CELVisitor<'input> + 'x))
    }
}

impl<'input> CELParserContext<'input> for TerminalNode<'input, CELParserContextType> {}
impl<'input> CELParserContext<'input> for ErrorNode<'input, CELParserContextType> {}

antlr4rust::tid! { impl<'input> TidAble<'input> for dyn CELParserContext<'input> + 'input }

antlr4rust::tid! { impl<'input> TidAble<'input> for dyn CELListener<'input> + 'input }

pub struct CELParserContextType;
antlr4rust::tid! {CELParserContextType}

impl<'input> ParserNodeType<'input> for CELParserContextType {
    type TF = LocalTokenFactory<'input>;
    type Type = dyn CELParserContext<'input> + 'input;
}

impl<'input, I> Deref for CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    type Target = BaseParserType<'input, I>;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<'input, I> DerefMut for CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

pub struct CELParserExt<'input> {
    _pd: PhantomData<&'input str>,
}

impl<'input> CELParserExt<'input> {}
antlr4rust::tid! { CELParserExt<'a> }

impl<'input> TokenAware<'input> for CELParserExt<'input> {
    type TF = LocalTokenFactory<'input>;
}

impl<'input, I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>>
    ParserRecog<'input, BaseParserType<'input, I>> for CELParserExt<'input>
{
}

impl<'input, I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>>
    Actions<'input, BaseParserType<'input, I>> for CELParserExt<'input>
{
    fn get_grammar_file_name(&self) -> &str {
        "CEL.g4"
    }

    fn get_rule_names(&self) -> &[&str] {
        &ruleNames
    }

    fn get_vocabulary(&self) -> &dyn Vocabulary {
        &**VOCABULARY
    }
    fn sempred(
        _localctx: Option<&(dyn CELParserContext<'input> + 'input)>,
        rule_index: i32,
        pred_index: i32,
        recog: &mut BaseParserType<'input, I>,
    ) -> bool {
        match rule_index {
            4 => CELParser::<'input, I>::relation_sempred(
                _localctx.and_then(|x| x.downcast_ref()),
                pred_index,
                recog,
            ),
            5 => CELParser::<'input, I>::calc_sempred(
                _localctx.and_then(|x| x.downcast_ref()),
                pred_index,
                recog,
            ),
            7 => CELParser::<'input, I>::member_sempred(
                _localctx.and_then(|x| x.downcast_ref()),
                pred_index,
                recog,
            ),
            _ => true,
        }
    }
}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    fn relation_sempred(
        _localctx: Option<&RelationContext<'input>>,
        pred_index: i32,
        recog: &mut <Self as Deref>::Target,
    ) -> bool {
        match pred_index {
            0 => recog.precpred(None, 1),
            _ => true,
        }
    }
    fn calc_sempred(
        _localctx: Option<&CalcContext<'input>>,
        pred_index: i32,
        recog: &mut <Self as Deref>::Target,
    ) -> bool {
        match pred_index {
            1 => recog.precpred(None, 2),
            2 => recog.precpred(None, 1),
            _ => true,
        }
    }
    fn member_sempred(
        _localctx: Option<&MemberContext<'input>>,
        pred_index: i32,
        recog: &mut <Self as Deref>::Target,
    ) -> bool {
        match pred_index {
            3 => recog.precpred(None, 3),
            4 => recog.precpred(None, 2),
            5 => recog.precpred(None, 1),
            _ => true,
        }
    }
}
//------------------- start ----------------
pub type StartContextAll<'input> = StartContext<'input>;

pub type StartContext<'input> = BaseParserRuleContext<'input, StartContextExt<'input>>;

#[derive(Clone)]
pub struct StartContextExt<'input> {
    pub e: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for StartContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for StartContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_start(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_start(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for StartContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_start(self);
    }
}

impl<'input> CustomRuleContext<'input> for StartContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_start
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_start }
}
antlr4rust::tid! {StartContextExt<'a>}

impl<'input> StartContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<StartContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            StartContextExt {
                e: None,

                ph: PhantomData,
            },
        ))
    }
}

pub trait StartContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<StartContextExt<'input>>
{
    /// Retrieves first TerminalNode corresponding to token EOF
    /// Returns `None` if there is no child corresponding to token EOF
    fn EOF(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_EOF, 0)
    }
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> StartContextAttrs<'input> for StartContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn start(&mut self) -> Result<Rc<StartContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = StartContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 0, RULE_start);
        let mut _localctx: Rc<StartContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                // InvokeRule expr
                recog.base.set_state(34);
                let tmp = recog.expr()?;
                cast_mut::<_, StartContext>(&mut _localctx).e = Some(tmp.clone());

                recog.base.set_state(35);
                recog.base.match_token(CEL_EOF, &mut recog.err_handler)?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- expr ----------------
pub type ExprContextAll<'input> = ExprContext<'input>;

pub type ExprContext<'input> = BaseParserRuleContext<'input, ExprContextExt<'input>>;

#[derive(Clone)]
pub struct ExprContextExt<'input> {
    pub e: Option<Rc<ConditionalOrContextAll<'input>>>,
    pub op: Option<TokenType<'input>>,
    pub e1: Option<Rc<ConditionalOrContextAll<'input>>>,
    pub e2: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for ExprContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for ExprContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_expr(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_expr(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for ExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_expr(self);
    }
}

impl<'input> CustomRuleContext<'input> for ExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_expr
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_expr }
}
antlr4rust::tid! {ExprContextExt<'a>}

impl<'input> ExprContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<ExprContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            ExprContextExt {
                op: None,
                e: None,
                e1: None,
                e2: None,

                ph: PhantomData,
            },
        ))
    }
}

pub trait ExprContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<ExprContextExt<'input>>
{
    fn conditionalOr_all(&self) -> Vec<Rc<ConditionalOrContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn conditionalOr(&self, i: usize) -> Option<Rc<ConditionalOrContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves first TerminalNode corresponding to token COLON
    /// Returns `None` if there is no child corresponding to token COLON
    fn COLON(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COLON, 0)
    }
    /// Retrieves first TerminalNode corresponding to token QUESTIONMARK
    /// Returns `None` if there is no child corresponding to token QUESTIONMARK
    fn QUESTIONMARK(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_QUESTIONMARK, 0)
    }
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> ExprContextAttrs<'input> for ExprContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn expr(&mut self) -> Result<Rc<ExprContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = ExprContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 2, RULE_expr);
        let mut _localctx: Rc<ExprContextAll> = _localctx;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                // InvokeRule conditionalOr
                recog.base.set_state(37);
                let tmp = recog.conditionalOr()?;
                cast_mut::<_, ExprContext>(&mut _localctx).e = Some(tmp.clone());

                recog.base.set_state(43);
                recog.err_handler.sync(&mut recog.base)?;
                _la = recog.base.input.la(1);
                if _la == CEL_QUESTIONMARK {
                    {
                        recog.base.set_state(38);
                        let tmp = recog
                            .base
                            .match_token(CEL_QUESTIONMARK, &mut recog.err_handler)?;
                        cast_mut::<_, ExprContext>(&mut _localctx).op = Some(tmp.clone());

                        // InvokeRule conditionalOr
                        recog.base.set_state(39);
                        let tmp = recog.conditionalOr()?;
                        cast_mut::<_, ExprContext>(&mut _localctx).e1 = Some(tmp.clone());

                        recog.base.set_state(40);
                        recog.base.match_token(CEL_COLON, &mut recog.err_handler)?;

                        // InvokeRule expr
                        recog.base.set_state(41);
                        let tmp = recog.expr()?;
                        cast_mut::<_, ExprContext>(&mut _localctx).e2 = Some(tmp.clone());
                    }
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- conditionalOr ----------------
pub type ConditionalOrContextAll<'input> = ConditionalOrContext<'input>;

pub type ConditionalOrContext<'input> =
    BaseParserRuleContext<'input, ConditionalOrContextExt<'input>>;

#[derive(Clone)]
pub struct ConditionalOrContextExt<'input> {
    pub e: Option<Rc<ConditionalAndContextAll<'input>>>,
    pub s9: Option<TokenType<'input>>,
    pub ops: Vec<TokenType<'input>>,
    pub conditionalAnd: Option<Rc<ConditionalAndContextAll<'input>>>,
    pub e1: Vec<Rc<ConditionalAndContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for ConditionalOrContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for ConditionalOrContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_conditionalOr(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_conditionalOr(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for ConditionalOrContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_conditionalOr(self);
    }
}

impl<'input> CustomRuleContext<'input> for ConditionalOrContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_conditionalOr
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_conditionalOr }
}
antlr4rust::tid! {ConditionalOrContextExt<'a>}

impl<'input> ConditionalOrContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<ConditionalOrContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            ConditionalOrContextExt {
                s9: None,
                ops: Vec::new(),
                e: None,
                conditionalAnd: None,
                e1: Vec::new(),

                ph: PhantomData,
            },
        ))
    }
}

pub trait ConditionalOrContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<ConditionalOrContextExt<'input>>
{
    fn conditionalAnd_all(&self) -> Vec<Rc<ConditionalAndContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn conditionalAnd(&self, i: usize) -> Option<Rc<ConditionalAndContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves all `TerminalNode`s corresponding to token LOGICAL_OR in current rule
    fn LOGICAL_OR_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token LOGICAL_OR, starting from 0.
    /// Returns `None` if number of children corresponding to token LOGICAL_OR is less or equal than `i`.
    fn LOGICAL_OR(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LOGICAL_OR, i)
    }
}

impl<'input> ConditionalOrContextAttrs<'input> for ConditionalOrContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn conditionalOr(&mut self) -> Result<Rc<ConditionalOrContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx =
            ConditionalOrContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_rule(_localctx.clone(), 4, RULE_conditionalOr);
        let mut _localctx: Rc<ConditionalOrContextAll> = _localctx;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                // InvokeRule conditionalAnd
                recog.base.set_state(45);
                let tmp = recog.conditionalAnd()?;
                cast_mut::<_, ConditionalOrContext>(&mut _localctx).e = Some(tmp.clone());

                recog.base.set_state(50);
                recog.err_handler.sync(&mut recog.base)?;
                _la = recog.base.input.la(1);
                while _la == CEL_LOGICAL_OR {
                    {
                        {
                            recog.base.set_state(46);
                            let tmp = recog
                                .base
                                .match_token(CEL_LOGICAL_OR, &mut recog.err_handler)?;
                            cast_mut::<_, ConditionalOrContext>(&mut _localctx).s9 =
                                Some(tmp.clone());

                            let temp = cast_mut::<_, ConditionalOrContext>(&mut _localctx)
                                .s9
                                .clone()
                                .unwrap();
                            cast_mut::<_, ConditionalOrContext>(&mut _localctx)
                                .ops
                                .push(temp);

                            // InvokeRule conditionalAnd
                            recog.base.set_state(47);
                            let tmp = recog.conditionalAnd()?;
                            cast_mut::<_, ConditionalOrContext>(&mut _localctx).conditionalAnd =
                                Some(tmp.clone());

                            let temp = cast_mut::<_, ConditionalOrContext>(&mut _localctx)
                                .conditionalAnd
                                .clone()
                                .unwrap();
                            cast_mut::<_, ConditionalOrContext>(&mut _localctx)
                                .e1
                                .push(temp);
                        }
                    }
                    recog.base.set_state(52);
                    recog.err_handler.sync(&mut recog.base)?;
                    _la = recog.base.input.la(1);
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- conditionalAnd ----------------
pub type ConditionalAndContextAll<'input> = ConditionalAndContext<'input>;

pub type ConditionalAndContext<'input> =
    BaseParserRuleContext<'input, ConditionalAndContextExt<'input>>;

#[derive(Clone)]
pub struct ConditionalAndContextExt<'input> {
    pub e: Option<Rc<RelationContextAll<'input>>>,
    pub s8: Option<TokenType<'input>>,
    pub ops: Vec<TokenType<'input>>,
    pub relation: Option<Rc<RelationContextAll<'input>>>,
    pub e1: Vec<Rc<RelationContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for ConditionalAndContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for ConditionalAndContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_conditionalAnd(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_conditionalAnd(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for ConditionalAndContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_conditionalAnd(self);
    }
}

impl<'input> CustomRuleContext<'input> for ConditionalAndContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_conditionalAnd
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_conditionalAnd }
}
antlr4rust::tid! {ConditionalAndContextExt<'a>}

impl<'input> ConditionalAndContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<ConditionalAndContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            ConditionalAndContextExt {
                s8: None,
                ops: Vec::new(),
                e: None,
                relation: None,
                e1: Vec::new(),

                ph: PhantomData,
            },
        ))
    }
}

pub trait ConditionalAndContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<ConditionalAndContextExt<'input>>
{
    fn relation_all(&self) -> Vec<Rc<RelationContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn relation(&self, i: usize) -> Option<Rc<RelationContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves all `TerminalNode`s corresponding to token LOGICAL_AND in current rule
    fn LOGICAL_AND_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token LOGICAL_AND, starting from 0.
    /// Returns `None` if number of children corresponding to token LOGICAL_AND is less or equal than `i`.
    fn LOGICAL_AND(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LOGICAL_AND, i)
    }
}

impl<'input> ConditionalAndContextAttrs<'input> for ConditionalAndContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn conditionalAnd(&mut self) -> Result<Rc<ConditionalAndContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx =
            ConditionalAndContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_rule(_localctx.clone(), 6, RULE_conditionalAnd);
        let mut _localctx: Rc<ConditionalAndContextAll> = _localctx;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                // InvokeRule relation
                recog.base.set_state(53);
                let tmp = recog.relation_rec(0)?;
                cast_mut::<_, ConditionalAndContext>(&mut _localctx).e = Some(tmp.clone());

                recog.base.set_state(58);
                recog.err_handler.sync(&mut recog.base)?;
                _la = recog.base.input.la(1);
                while _la == CEL_LOGICAL_AND {
                    {
                        {
                            recog.base.set_state(54);
                            let tmp = recog
                                .base
                                .match_token(CEL_LOGICAL_AND, &mut recog.err_handler)?;
                            cast_mut::<_, ConditionalAndContext>(&mut _localctx).s8 =
                                Some(tmp.clone());

                            let temp = cast_mut::<_, ConditionalAndContext>(&mut _localctx)
                                .s8
                                .clone()
                                .unwrap();
                            cast_mut::<_, ConditionalAndContext>(&mut _localctx)
                                .ops
                                .push(temp);

                            // InvokeRule relation
                            recog.base.set_state(55);
                            let tmp = recog.relation_rec(0)?;
                            cast_mut::<_, ConditionalAndContext>(&mut _localctx).relation =
                                Some(tmp.clone());

                            let temp = cast_mut::<_, ConditionalAndContext>(&mut _localctx)
                                .relation
                                .clone()
                                .unwrap();
                            cast_mut::<_, ConditionalAndContext>(&mut _localctx)
                                .e1
                                .push(temp);
                        }
                    }
                    recog.base.set_state(60);
                    recog.err_handler.sync(&mut recog.base)?;
                    _la = recog.base.input.la(1);
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- relation ----------------
pub type RelationContextAll<'input> = RelationContext<'input>;

pub type RelationContext<'input> = BaseParserRuleContext<'input, RelationContextExt<'input>>;

#[derive(Clone)]
pub struct RelationContextExt<'input> {
    pub op: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for RelationContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for RelationContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_relation(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_relation(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for RelationContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_relation(self);
    }
}

impl<'input> CustomRuleContext<'input> for RelationContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_relation
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_relation }
}
antlr4rust::tid! {RelationContextExt<'a>}

impl<'input> RelationContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<RelationContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            RelationContextExt {
                op: None,

                ph: PhantomData,
            },
        ))
    }
}

pub trait RelationContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<RelationContextExt<'input>>
{
    fn calc(&self) -> Option<Rc<CalcContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn relation_all(&self) -> Vec<Rc<RelationContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn relation(&self, i: usize) -> Option<Rc<RelationContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves first TerminalNode corresponding to token LESS
    /// Returns `None` if there is no child corresponding to token LESS
    fn LESS(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LESS, 0)
    }
    /// Retrieves first TerminalNode corresponding to token LESS_EQUALS
    /// Returns `None` if there is no child corresponding to token LESS_EQUALS
    fn LESS_EQUALS(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LESS_EQUALS, 0)
    }
    /// Retrieves first TerminalNode corresponding to token GREATER_EQUALS
    /// Returns `None` if there is no child corresponding to token GREATER_EQUALS
    fn GREATER_EQUALS(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_GREATER_EQUALS, 0)
    }
    /// Retrieves first TerminalNode corresponding to token GREATER
    /// Returns `None` if there is no child corresponding to token GREATER
    fn GREATER(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_GREATER, 0)
    }
    /// Retrieves first TerminalNode corresponding to token EQUALS
    /// Returns `None` if there is no child corresponding to token EQUALS
    fn EQUALS(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_EQUALS, 0)
    }
    /// Retrieves first TerminalNode corresponding to token NOT_EQUALS
    /// Returns `None` if there is no child corresponding to token NOT_EQUALS
    fn NOT_EQUALS(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_NOT_EQUALS, 0)
    }
    /// Retrieves first TerminalNode corresponding to token IN
    /// Returns `None` if there is no child corresponding to token IN
    fn IN(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_IN, 0)
    }
}

impl<'input> RelationContextAttrs<'input> for RelationContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn relation(&mut self) -> Result<Rc<RelationContextAll<'input>>, ANTLRError> {
        self.relation_rec(0)
    }

    fn relation_rec(&mut self, _p: i32) -> Result<Rc<RelationContextAll<'input>>, ANTLRError> {
        let recog = self;
        let _parentctx = recog.ctx.take();
        let _parentState = recog.base.get_state();
        let mut _localctx = RelationContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_recursion_rule(_localctx.clone(), 8, RULE_relation, _p);
        let mut _localctx: Rc<RelationContextAll> = _localctx;
        let mut _prevctx = _localctx.clone();
        let _startState = 8;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: i32;
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                {
                    // InvokeRule calc
                    recog.base.set_state(62);
                    recog.calc_rec(0)?;
                }
                let tmp = recog.input.lt(-1).cloned();
                recog.ctx.as_ref().unwrap().set_stop(tmp);
                recog.base.set_state(69);
                recog.err_handler.sync(&mut recog.base)?;
                _alt = recog.interpreter.adaptive_predict(3, &mut recog.base)?;
                while { _alt != 2 && _alt != INVALID_ALT } {
                    if _alt == 1 {
                        recog.trigger_exit_rule_event()?;
                        _prevctx = _localctx.clone();
                        {
                            {
                                // recRuleAltStartAction
                                let mut tmp =
                                    RelationContextExt::new(_parentctx.clone(), _parentState);
                                recog.push_new_recursion_context(
                                    tmp.clone(),
                                    _startState,
                                    RULE_relation,
                                )?;
                                _localctx = tmp;
                                recog.base.set_state(64);
                                if !({
                                    let _localctx = Some(_localctx.clone());
                                    recog.precpred(None, 1)
                                }) {
                                    Err(FailedPredicateError::new(
                                        &mut recog.base,
                                        Some("recog.precpred(None, 1)".to_owned()),
                                        None,
                                    ))?;
                                }
                                recog.base.set_state(65);
                                cast_mut::<_, RelationContext>(&mut _localctx).op =
                                    recog.base.input.lt(1).cloned();

                                _la = recog.base.input.la(1);
                                if { !(((_la) & !0x3f) == 0 && ((1usize << _la) & 254) != 0) } {
                                    let tmp = recog.err_handler.recover_inline(&mut recog.base)?;
                                    cast_mut::<_, RelationContext>(&mut _localctx).op =
                                        Some(tmp.clone());
                                } else {
                                    if recog.base.input.la(1) == TOKEN_EOF {
                                        recog.base.matched_eof = true
                                    };
                                    recog.err_handler.report_match(&mut recog.base);
                                    recog.base.consume(&mut recog.err_handler);
                                }
                                // InvokeRule relation
                                recog.base.set_state(66);
                                recog.relation_rec(2)?;
                            }
                        }
                    }
                    recog.base.set_state(71);
                    recog.err_handler.sync(&mut recog.base)?;
                    _alt = recog.interpreter.adaptive_predict(3, &mut recog.base)?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.unroll_recursion_context(_parentctx)?;

        Ok(_localctx)
    }
}
//------------------- calc ----------------
pub type CalcContextAll<'input> = CalcContext<'input>;

pub type CalcContext<'input> = BaseParserRuleContext<'input, CalcContextExt<'input>>;

#[derive(Clone)]
pub struct CalcContextExt<'input> {
    pub op: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for CalcContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for CalcContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_calc(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_calc(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for CalcContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_calc(self);
    }
}

impl<'input> CustomRuleContext<'input> for CalcContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_calc
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_calc }
}
antlr4rust::tid! {CalcContextExt<'a>}

impl<'input> CalcContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<CalcContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            CalcContextExt {
                op: None,

                ph: PhantomData,
            },
        ))
    }
}

pub trait CalcContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<CalcContextExt<'input>>
{
    fn unary(&self) -> Option<Rc<UnaryContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    fn calc_all(&self) -> Vec<Rc<CalcContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn calc(&self, i: usize) -> Option<Rc<CalcContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves first TerminalNode corresponding to token STAR
    /// Returns `None` if there is no child corresponding to token STAR
    fn STAR(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_STAR, 0)
    }
    /// Retrieves first TerminalNode corresponding to token SLASH
    /// Returns `None` if there is no child corresponding to token SLASH
    fn SLASH(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_SLASH, 0)
    }
    /// Retrieves first TerminalNode corresponding to token PERCENT
    /// Returns `None` if there is no child corresponding to token PERCENT
    fn PERCENT(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_PERCENT, 0)
    }
    /// Retrieves first TerminalNode corresponding to token PLUS
    /// Returns `None` if there is no child corresponding to token PLUS
    fn PLUS(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_PLUS, 0)
    }
    /// Retrieves first TerminalNode corresponding to token MINUS
    /// Returns `None` if there is no child corresponding to token MINUS
    fn MINUS(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_MINUS, 0)
    }
}

impl<'input> CalcContextAttrs<'input> for CalcContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn calc(&mut self) -> Result<Rc<CalcContextAll<'input>>, ANTLRError> {
        self.calc_rec(0)
    }

    fn calc_rec(&mut self, _p: i32) -> Result<Rc<CalcContextAll<'input>>, ANTLRError> {
        let recog = self;
        let _parentctx = recog.ctx.take();
        let _parentState = recog.base.get_state();
        let mut _localctx = CalcContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_recursion_rule(_localctx.clone(), 10, RULE_calc, _p);
        let mut _localctx: Rc<CalcContextAll> = _localctx;
        let mut _prevctx = _localctx.clone();
        let _startState = 10;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: i32;
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                {
                    // InvokeRule unary
                    recog.base.set_state(73);
                    recog.unary()?;
                }
                let tmp = recog.input.lt(-1).cloned();
                recog.ctx.as_ref().unwrap().set_stop(tmp);
                recog.base.set_state(83);
                recog.err_handler.sync(&mut recog.base)?;
                _alt = recog.interpreter.adaptive_predict(5, &mut recog.base)?;
                while { _alt != 2 && _alt != INVALID_ALT } {
                    if _alt == 1 {
                        recog.trigger_exit_rule_event()?;
                        _prevctx = _localctx.clone();
                        {
                            recog.base.set_state(81);
                            recog.err_handler.sync(&mut recog.base)?;
                            match recog.interpreter.adaptive_predict(4, &mut recog.base)? {
                                1 => {
                                    {
                                        // recRuleAltStartAction
                                        let mut tmp =
                                            CalcContextExt::new(_parentctx.clone(), _parentState);
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_calc,
                                        )?;
                                        _localctx = tmp;
                                        recog.base.set_state(75);
                                        if !({
                                            let _localctx = Some(_localctx.clone());
                                            recog.precpred(None, 2)
                                        }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 2)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(76);
                                        cast_mut::<_, CalcContext>(&mut _localctx).op =
                                            recog.base.input.lt(1).cloned();

                                        _la = recog.base.input.la(1);
                                        if {
                                            !(((_la) & !0x3f) == 0
                                                && ((1usize << _la) & 58720256) != 0)
                                        } {
                                            let tmp = recog
                                                .err_handler
                                                .recover_inline(&mut recog.base)?;
                                            cast_mut::<_, CalcContext>(&mut _localctx).op =
                                                Some(tmp.clone());
                                        } else {
                                            if recog.base.input.la(1) == TOKEN_EOF {
                                                recog.base.matched_eof = true
                                            };
                                            recog.err_handler.report_match(&mut recog.base);
                                            recog.base.consume(&mut recog.err_handler);
                                        }
                                        // InvokeRule calc
                                        recog.base.set_state(77);
                                        recog.calc_rec(3)?;
                                    }
                                }
                                2 => {
                                    {
                                        // recRuleAltStartAction
                                        let mut tmp =
                                            CalcContextExt::new(_parentctx.clone(), _parentState);
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_calc,
                                        )?;
                                        _localctx = tmp;
                                        recog.base.set_state(78);
                                        if !({
                                            let _localctx = Some(_localctx.clone());
                                            recog.precpred(None, 1)
                                        }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 1)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(79);
                                        cast_mut::<_, CalcContext>(&mut _localctx).op =
                                            recog.base.input.lt(1).cloned();

                                        _la = recog.base.input.la(1);
                                        if { !(_la == CEL_MINUS || _la == CEL_PLUS) } {
                                            let tmp = recog
                                                .err_handler
                                                .recover_inline(&mut recog.base)?;
                                            cast_mut::<_, CalcContext>(&mut _localctx).op =
                                                Some(tmp.clone());
                                        } else {
                                            if recog.base.input.la(1) == TOKEN_EOF {
                                                recog.base.matched_eof = true
                                            };
                                            recog.err_handler.report_match(&mut recog.base);
                                            recog.base.consume(&mut recog.err_handler);
                                        }
                                        // InvokeRule calc
                                        recog.base.set_state(80);
                                        recog.calc_rec(2)?;
                                    }
                                }

                                _ => {}
                            }
                        }
                    }
                    recog.base.set_state(85);
                    recog.err_handler.sync(&mut recog.base)?;
                    _alt = recog.interpreter.adaptive_predict(5, &mut recog.base)?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.unroll_recursion_context(_parentctx)?;

        Ok(_localctx)
    }
}
//------------------- unary ----------------
#[derive(Debug)]
pub enum UnaryContextAll<'input> {
    LogicalNotContext(LogicalNotContext<'input>),
    MemberExprContext(MemberExprContext<'input>),
    NegateContext(NegateContext<'input>),
    Error(UnaryContext<'input>),
}
antlr4rust::tid! {UnaryContextAll<'a>}

impl<'input> antlr4rust::parser_rule_context::DerefSeal for UnaryContextAll<'input> {}

impl<'input> CELParserContext<'input> for UnaryContextAll<'input> {}

impl<'input> Deref for UnaryContextAll<'input> {
    type Target = dyn UnaryContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use UnaryContextAll::*;
        match self {
            LogicalNotContext(inner) => inner,
            MemberExprContext(inner) => inner,
            NegateContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for UnaryContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for UnaryContextAll<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().exit(listener)
    }
}

pub type UnaryContext<'input> = BaseParserRuleContext<'input, UnaryContextExt<'input>>;

#[derive(Clone)]
pub struct UnaryContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for UnaryContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for UnaryContext<'input> {}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for UnaryContext<'input> {}

impl<'input> CustomRuleContext<'input> for UnaryContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_unary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_unary }
}
antlr4rust::tid! {UnaryContextExt<'a>}

impl<'input> UnaryContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<UnaryContextAll<'input>> {
        Rc::new(UnaryContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                UnaryContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait UnaryContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<UnaryContextExt<'input>>
{
}

impl<'input> UnaryContextAttrs<'input> for UnaryContext<'input> {}

pub type LogicalNotContext<'input> = BaseParserRuleContext<'input, LogicalNotContextExt<'input>>;

pub trait LogicalNotContextAttrs<'input>: CELParserContext<'input> {
    fn member(&self) -> Option<Rc<MemberContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    /// Retrieves all `TerminalNode`s corresponding to token EXCLAM in current rule
    fn EXCLAM_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token EXCLAM, starting from 0.
    /// Returns `None` if number of children corresponding to token EXCLAM is less or equal than `i`.
    fn EXCLAM(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_EXCLAM, i)
    }
}

impl<'input> LogicalNotContextAttrs<'input> for LogicalNotContext<'input> {}

pub struct LogicalNotContextExt<'input> {
    base: UnaryContextExt<'input>,
    pub s19: Option<TokenType<'input>>,
    pub ops: Vec<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {LogicalNotContextExt<'a>}

impl<'input> CELParserContext<'input> for LogicalNotContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for LogicalNotContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_LogicalNot(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_LogicalNot(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for LogicalNotContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_LogicalNot(self);
    }
}

impl<'input> CustomRuleContext<'input> for LogicalNotContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_unary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_unary }
}

impl<'input> Borrow<UnaryContextExt<'input>> for LogicalNotContext<'input> {
    fn borrow(&self) -> &UnaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<UnaryContextExt<'input>> for LogicalNotContext<'input> {
    fn borrow_mut(&mut self) -> &mut UnaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> UnaryContextAttrs<'input> for LogicalNotContext<'input> {}

impl<'input> LogicalNotContextExt<'input> {
    fn new(ctx: &dyn UnaryContextAttrs<'input>) -> Rc<UnaryContextAll<'input>> {
        Rc::new(UnaryContextAll::LogicalNotContext(
            BaseParserRuleContext::copy_from(
                ctx,
                LogicalNotContextExt {
                    s19: None,
                    ops: Vec::new(),
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type MemberExprContext<'input> = BaseParserRuleContext<'input, MemberExprContextExt<'input>>;

pub trait MemberExprContextAttrs<'input>: CELParserContext<'input> {
    fn member(&self) -> Option<Rc<MemberContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> MemberExprContextAttrs<'input> for MemberExprContext<'input> {}

pub struct MemberExprContextExt<'input> {
    base: UnaryContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {MemberExprContextExt<'a>}

impl<'input> CELParserContext<'input> for MemberExprContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for MemberExprContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_MemberExpr(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_MemberExpr(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for MemberExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_MemberExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for MemberExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_unary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_unary }
}

impl<'input> Borrow<UnaryContextExt<'input>> for MemberExprContext<'input> {
    fn borrow(&self) -> &UnaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<UnaryContextExt<'input>> for MemberExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut UnaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> UnaryContextAttrs<'input> for MemberExprContext<'input> {}

impl<'input> MemberExprContextExt<'input> {
    fn new(ctx: &dyn UnaryContextAttrs<'input>) -> Rc<UnaryContextAll<'input>> {
        Rc::new(UnaryContextAll::MemberExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                MemberExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type NegateContext<'input> = BaseParserRuleContext<'input, NegateContextExt<'input>>;

pub trait NegateContextAttrs<'input>: CELParserContext<'input> {
    fn member(&self) -> Option<Rc<MemberContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    /// Retrieves all `TerminalNode`s corresponding to token MINUS in current rule
    fn MINUS_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token MINUS, starting from 0.
    /// Returns `None` if number of children corresponding to token MINUS is less or equal than `i`.
    fn MINUS(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_MINUS, i)
    }
}

impl<'input> NegateContextAttrs<'input> for NegateContext<'input> {}

pub struct NegateContextExt<'input> {
    base: UnaryContextExt<'input>,
    pub s18: Option<TokenType<'input>>,
    pub ops: Vec<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {NegateContextExt<'a>}

impl<'input> CELParserContext<'input> for NegateContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for NegateContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Negate(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Negate(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for NegateContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Negate(self);
    }
}

impl<'input> CustomRuleContext<'input> for NegateContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_unary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_unary }
}

impl<'input> Borrow<UnaryContextExt<'input>> for NegateContext<'input> {
    fn borrow(&self) -> &UnaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<UnaryContextExt<'input>> for NegateContext<'input> {
    fn borrow_mut(&mut self) -> &mut UnaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> UnaryContextAttrs<'input> for NegateContext<'input> {}

impl<'input> NegateContextExt<'input> {
    fn new(ctx: &dyn UnaryContextAttrs<'input>) -> Rc<UnaryContextAll<'input>> {
        Rc::new(UnaryContextAll::NegateContext(
            BaseParserRuleContext::copy_from(
                ctx,
                NegateContextExt {
                    s18: None,
                    ops: Vec::new(),
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn unary(&mut self) -> Result<Rc<UnaryContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = UnaryContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 12, RULE_unary);
        let mut _localctx: Rc<UnaryContextAll> = _localctx;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: i32;
            recog.base.set_state(99);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(8, &mut recog.base)? {
                1 => {
                    let tmp = MemberExprContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1)?;
                    _localctx = tmp;
                    {
                        // InvokeRule member
                        recog.base.set_state(86);
                        recog.member_rec(0)?;
                    }
                }
                2 => {
                    let tmp = LogicalNotContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(88);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        loop {
                            {
                                {
                                    recog.base.set_state(87);
                                    let tmp = recog
                                        .base
                                        .match_token(CEL_EXCLAM, &mut recog.err_handler)?;
                                    if let UnaryContextAll::LogicalNotContext(ctx) =
                                        cast_mut::<_, UnaryContextAll>(&mut _localctx)
                                    {
                                        ctx.s19 = Some(tmp.clone());
                                    } else {
                                        unreachable!("cant cast");
                                    }

                                    let temp = if let UnaryContextAll::LogicalNotContext(ctx) =
                                        cast_mut::<_, UnaryContextAll>(&mut _localctx)
                                    {
                                        ctx.s19.clone().unwrap()
                                    } else {
                                        unreachable!("cant cast");
                                    };
                                    if let UnaryContextAll::LogicalNotContext(ctx) =
                                        cast_mut::<_, UnaryContextAll>(&mut _localctx)
                                    {
                                        ctx.ops.push(temp);
                                    } else {
                                        unreachable!("cant cast");
                                    }
                                }
                            }
                            recog.base.set_state(90);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                            if !(_la == CEL_EXCLAM) {
                                break;
                            }
                        }
                        // InvokeRule member
                        recog.base.set_state(92);
                        recog.member_rec(0)?;
                    }
                }
                3 => {
                    let tmp = NegateContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(94);
                        recog.err_handler.sync(&mut recog.base)?;
                        _alt = 1;
                        loop {
                            match _alt {
                                x if x == 1 => {
                                    recog.base.set_state(93);
                                    let tmp = recog
                                        .base
                                        .match_token(CEL_MINUS, &mut recog.err_handler)?;
                                    if let UnaryContextAll::NegateContext(ctx) =
                                        cast_mut::<_, UnaryContextAll>(&mut _localctx)
                                    {
                                        ctx.s18 = Some(tmp.clone());
                                    } else {
                                        unreachable!("cant cast");
                                    }

                                    let temp = if let UnaryContextAll::NegateContext(ctx) =
                                        cast_mut::<_, UnaryContextAll>(&mut _localctx)
                                    {
                                        ctx.s18.clone().unwrap()
                                    } else {
                                        unreachable!("cant cast");
                                    };
                                    if let UnaryContextAll::NegateContext(ctx) =
                                        cast_mut::<_, UnaryContextAll>(&mut _localctx)
                                    {
                                        ctx.ops.push(temp);
                                    } else {
                                        unreachable!("cant cast");
                                    }
                                }

                                _ => Err(ANTLRError::NoAltError(NoViableAltError::new(
                                    &mut recog.base,
                                )))?,
                            }
                            recog.base.set_state(96);
                            recog.err_handler.sync(&mut recog.base)?;
                            _alt = recog.interpreter.adaptive_predict(7, &mut recog.base)?;
                            if _alt == 2 || _alt == INVALID_ALT {
                                break;
                            }
                        }
                        // InvokeRule member
                        recog.base.set_state(98);
                        recog.member_rec(0)?;
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- member ----------------
#[derive(Debug)]
pub enum MemberContextAll<'input> {
    MemberCallContext(MemberCallContext<'input>),
    SelectContext(SelectContext<'input>),
    PrimaryExprContext(PrimaryExprContext<'input>),
    IndexContext(IndexContext<'input>),
    Error(MemberContext<'input>),
}
antlr4rust::tid! {MemberContextAll<'a>}

impl<'input> antlr4rust::parser_rule_context::DerefSeal for MemberContextAll<'input> {}

impl<'input> CELParserContext<'input> for MemberContextAll<'input> {}

impl<'input> Deref for MemberContextAll<'input> {
    type Target = dyn MemberContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use MemberContextAll::*;
        match self {
            MemberCallContext(inner) => inner,
            SelectContext(inner) => inner,
            PrimaryExprContext(inner) => inner,
            IndexContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for MemberContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for MemberContextAll<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().exit(listener)
    }
}

pub type MemberContext<'input> = BaseParserRuleContext<'input, MemberContextExt<'input>>;

#[derive(Clone)]
pub struct MemberContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for MemberContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for MemberContext<'input> {}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for MemberContext<'input> {}

impl<'input> CustomRuleContext<'input> for MemberContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_member
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_member }
}
antlr4rust::tid! {MemberContextExt<'a>}

impl<'input> MemberContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<MemberContextAll<'input>> {
        Rc::new(MemberContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                MemberContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait MemberContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<MemberContextExt<'input>>
{
}

impl<'input> MemberContextAttrs<'input> for MemberContext<'input> {}

pub type MemberCallContext<'input> = BaseParserRuleContext<'input, MemberCallContextExt<'input>>;

pub trait MemberCallContextAttrs<'input>: CELParserContext<'input> {
    fn member(&self) -> Option<Rc<MemberContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    /// Retrieves first TerminalNode corresponding to token RPAREN
    /// Returns `None` if there is no child corresponding to token RPAREN
    fn RPAREN(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_RPAREN, 0)
    }
    /// Retrieves first TerminalNode corresponding to token DOT
    /// Returns `None` if there is no child corresponding to token DOT
    fn DOT(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_DOT, 0)
    }
    /// Retrieves first TerminalNode corresponding to token IDENTIFIER
    /// Returns `None` if there is no child corresponding to token IDENTIFIER
    fn IDENTIFIER(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_IDENTIFIER, 0)
    }
    /// Retrieves first TerminalNode corresponding to token LPAREN
    /// Returns `None` if there is no child corresponding to token LPAREN
    fn LPAREN(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LPAREN, 0)
    }
    fn exprList(&self) -> Option<Rc<ExprListContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> MemberCallContextAttrs<'input> for MemberCallContext<'input> {}

pub struct MemberCallContextExt<'input> {
    base: MemberContextExt<'input>,
    pub op: Option<TokenType<'input>>,
    pub id: Option<TokenType<'input>>,
    pub open: Option<TokenType<'input>>,
    pub args: Option<Rc<ExprListContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {MemberCallContextExt<'a>}

impl<'input> CELParserContext<'input> for MemberCallContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for MemberCallContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_MemberCall(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_MemberCall(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for MemberCallContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_MemberCall(self);
    }
}

impl<'input> CustomRuleContext<'input> for MemberCallContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_member
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_member }
}

impl<'input> Borrow<MemberContextExt<'input>> for MemberCallContext<'input> {
    fn borrow(&self) -> &MemberContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<MemberContextExt<'input>> for MemberCallContext<'input> {
    fn borrow_mut(&mut self) -> &mut MemberContextExt<'input> {
        &mut self.base
    }
}

impl<'input> MemberContextAttrs<'input> for MemberCallContext<'input> {}

impl<'input> MemberCallContextExt<'input> {
    fn new(ctx: &dyn MemberContextAttrs<'input>) -> Rc<MemberContextAll<'input>> {
        Rc::new(MemberContextAll::MemberCallContext(
            BaseParserRuleContext::copy_from(
                ctx,
                MemberCallContextExt {
                    op: None,
                    id: None,
                    open: None,
                    args: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type SelectContext<'input> = BaseParserRuleContext<'input, SelectContextExt<'input>>;

pub trait SelectContextAttrs<'input>: CELParserContext<'input> {
    fn member(&self) -> Option<Rc<MemberContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    /// Retrieves first TerminalNode corresponding to token DOT
    /// Returns `None` if there is no child corresponding to token DOT
    fn DOT(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_DOT, 0)
    }
    fn escapeIdent(&self) -> Option<Rc<EscapeIdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    /// Retrieves first TerminalNode corresponding to token QUESTIONMARK
    /// Returns `None` if there is no child corresponding to token QUESTIONMARK
    fn QUESTIONMARK(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_QUESTIONMARK, 0)
    }
}

impl<'input> SelectContextAttrs<'input> for SelectContext<'input> {}

pub struct SelectContextExt<'input> {
    base: MemberContextExt<'input>,
    pub op: Option<TokenType<'input>>,
    pub opt: Option<TokenType<'input>>,
    pub id: Option<Rc<EscapeIdentContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {SelectContextExt<'a>}

impl<'input> CELParserContext<'input> for SelectContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for SelectContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Select(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Select(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for SelectContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Select(self);
    }
}

impl<'input> CustomRuleContext<'input> for SelectContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_member
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_member }
}

impl<'input> Borrow<MemberContextExt<'input>> for SelectContext<'input> {
    fn borrow(&self) -> &MemberContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<MemberContextExt<'input>> for SelectContext<'input> {
    fn borrow_mut(&mut self) -> &mut MemberContextExt<'input> {
        &mut self.base
    }
}

impl<'input> MemberContextAttrs<'input> for SelectContext<'input> {}

impl<'input> SelectContextExt<'input> {
    fn new(ctx: &dyn MemberContextAttrs<'input>) -> Rc<MemberContextAll<'input>> {
        Rc::new(MemberContextAll::SelectContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SelectContextExt {
                    op: None,
                    opt: None,
                    id: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type PrimaryExprContext<'input> = BaseParserRuleContext<'input, PrimaryExprContextExt<'input>>;

pub trait PrimaryExprContextAttrs<'input>: CELParserContext<'input> {
    fn primary(&self) -> Option<Rc<PrimaryContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> PrimaryExprContextAttrs<'input> for PrimaryExprContext<'input> {}

pub struct PrimaryExprContextExt<'input> {
    base: MemberContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {PrimaryExprContextExt<'a>}

impl<'input> CELParserContext<'input> for PrimaryExprContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for PrimaryExprContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_PrimaryExpr(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_PrimaryExpr(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for PrimaryExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_PrimaryExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for PrimaryExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_member
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_member }
}

impl<'input> Borrow<MemberContextExt<'input>> for PrimaryExprContext<'input> {
    fn borrow(&self) -> &MemberContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<MemberContextExt<'input>> for PrimaryExprContext<'input> {
    fn borrow_mut(&mut self) -> &mut MemberContextExt<'input> {
        &mut self.base
    }
}

impl<'input> MemberContextAttrs<'input> for PrimaryExprContext<'input> {}

impl<'input> PrimaryExprContextExt<'input> {
    fn new(ctx: &dyn MemberContextAttrs<'input>) -> Rc<MemberContextAll<'input>> {
        Rc::new(MemberContextAll::PrimaryExprContext(
            BaseParserRuleContext::copy_from(
                ctx,
                PrimaryExprContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type IndexContext<'input> = BaseParserRuleContext<'input, IndexContextExt<'input>>;

pub trait IndexContextAttrs<'input>: CELParserContext<'input> {
    fn member(&self) -> Option<Rc<MemberContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    /// Retrieves first TerminalNode corresponding to token RPRACKET
    /// Returns `None` if there is no child corresponding to token RPRACKET
    fn RPRACKET(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_RPRACKET, 0)
    }
    /// Retrieves first TerminalNode corresponding to token LBRACKET
    /// Returns `None` if there is no child corresponding to token LBRACKET
    fn LBRACKET(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LBRACKET, 0)
    }
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    /// Retrieves first TerminalNode corresponding to token QUESTIONMARK
    /// Returns `None` if there is no child corresponding to token QUESTIONMARK
    fn QUESTIONMARK(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_QUESTIONMARK, 0)
    }
}

impl<'input> IndexContextAttrs<'input> for IndexContext<'input> {}

pub struct IndexContextExt<'input> {
    base: MemberContextExt<'input>,
    pub op: Option<TokenType<'input>>,
    pub opt: Option<TokenType<'input>>,
    pub index: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {IndexContextExt<'a>}

impl<'input> CELParserContext<'input> for IndexContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for IndexContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Index(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Index(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for IndexContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Index(self);
    }
}

impl<'input> CustomRuleContext<'input> for IndexContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_member
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_member }
}

impl<'input> Borrow<MemberContextExt<'input>> for IndexContext<'input> {
    fn borrow(&self) -> &MemberContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<MemberContextExt<'input>> for IndexContext<'input> {
    fn borrow_mut(&mut self) -> &mut MemberContextExt<'input> {
        &mut self.base
    }
}

impl<'input> MemberContextAttrs<'input> for IndexContext<'input> {}

impl<'input> IndexContextExt<'input> {
    fn new(ctx: &dyn MemberContextAttrs<'input>) -> Rc<MemberContextAll<'input>> {
        Rc::new(MemberContextAll::IndexContext(
            BaseParserRuleContext::copy_from(
                ctx,
                IndexContextExt {
                    op: None,
                    opt: None,
                    index: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn member(&mut self) -> Result<Rc<MemberContextAll<'input>>, ANTLRError> {
        self.member_rec(0)
    }

    fn member_rec(&mut self, _p: i32) -> Result<Rc<MemberContextAll<'input>>, ANTLRError> {
        let recog = self;
        let _parentctx = recog.ctx.take();
        let _parentState = recog.base.get_state();
        let mut _localctx = MemberContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_recursion_rule(_localctx.clone(), 14, RULE_member, _p);
        let mut _localctx: Rc<MemberContextAll> = _localctx;
        let mut _prevctx = _localctx.clone();
        let _startState = 14;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: i32;
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                {
                    let mut tmp = PrimaryExprContextExt::new(&**_localctx);
                    recog.ctx = Some(tmp.clone());
                    _localctx = tmp;
                    _prevctx = _localctx.clone();

                    // InvokeRule primary
                    recog.base.set_state(102);
                    recog.primary()?;
                }
                let tmp = recog.input.lt(-1).cloned();
                recog.ctx.as_ref().unwrap().set_stop(tmp);
                recog.base.set_state(128);
                recog.err_handler.sync(&mut recog.base)?;
                _alt = recog.interpreter.adaptive_predict(13, &mut recog.base)?;
                while { _alt != 2 && _alt != INVALID_ALT } {
                    if _alt == 1 {
                        recog.trigger_exit_rule_event()?;
                        _prevctx = _localctx.clone();
                        {
                            recog.base.set_state(126);
                            recog.err_handler.sync(&mut recog.base)?;
                            match recog.interpreter.adaptive_predict(12, &mut recog.base)? {
                                1 => {
                                    {
                                        // recRuleLabeledAltStartAction
                                        let mut tmp =
                                            SelectContextExt::new(&**MemberContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_member,
                                        )?;
                                        _localctx = tmp;
                                        recog.base.set_state(104);
                                        if !({
                                            let _localctx = Some(_localctx.clone());
                                            recog.precpred(None, 3)
                                        }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 3)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(105);
                                        let tmp = recog
                                            .base
                                            .match_token(CEL_DOT, &mut recog.err_handler)?;
                                        if let MemberContextAll::SelectContext(ctx) =
                                            cast_mut::<_, MemberContextAll>(&mut _localctx)
                                        {
                                            ctx.op = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        recog.base.set_state(107);
                                        recog.err_handler.sync(&mut recog.base)?;
                                        _la = recog.base.input.la(1);
                                        if _la == CEL_QUESTIONMARK {
                                            {
                                                recog.base.set_state(106);
                                                let tmp = recog.base.match_token(
                                                    CEL_QUESTIONMARK,
                                                    &mut recog.err_handler,
                                                )?;
                                                if let MemberContextAll::SelectContext(ctx) =
                                                    cast_mut::<_, MemberContextAll>(&mut _localctx)
                                                {
                                                    ctx.opt = Some(tmp.clone());
                                                } else {
                                                    unreachable!("cant cast");
                                                }
                                            }
                                        }

                                        // InvokeRule escapeIdent
                                        recog.base.set_state(109);
                                        let tmp = recog.escapeIdent()?;
                                        if let MemberContextAll::SelectContext(ctx) =
                                            cast_mut::<_, MemberContextAll>(&mut _localctx)
                                        {
                                            ctx.id = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }
                                    }
                                }
                                2 => {
                                    {
                                        // recRuleLabeledAltStartAction
                                        let mut tmp =
                                            MemberCallContextExt::new(&**MemberContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_member,
                                        )?;
                                        _localctx = tmp;
                                        recog.base.set_state(110);
                                        if !({
                                            let _localctx = Some(_localctx.clone());
                                            recog.precpred(None, 2)
                                        }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 2)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(111);
                                        let tmp = recog
                                            .base
                                            .match_token(CEL_DOT, &mut recog.err_handler)?;
                                        if let MemberContextAll::MemberCallContext(ctx) =
                                            cast_mut::<_, MemberContextAll>(&mut _localctx)
                                        {
                                            ctx.op = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        recog.base.set_state(112);
                                        let tmp = recog
                                            .base
                                            .match_token(CEL_IDENTIFIER, &mut recog.err_handler)?;
                                        if let MemberContextAll::MemberCallContext(ctx) =
                                            cast_mut::<_, MemberContextAll>(&mut _localctx)
                                        {
                                            ctx.id = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        recog.base.set_state(113);
                                        let tmp = recog
                                            .base
                                            .match_token(CEL_LPAREN, &mut recog.err_handler)?;
                                        if let MemberContextAll::MemberCallContext(ctx) =
                                            cast_mut::<_, MemberContextAll>(&mut _localctx)
                                        {
                                            ctx.open = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        recog.base.set_state(115);
                                        recog.err_handler.sync(&mut recog.base)?;
                                        _la = recog.base.input.la(1);
                                        if ((_la - 10) & !0x3f) == 0
                                            && ((1usize << (_la - 10)) & 132580181) != 0
                                        {
                                            {
                                                // InvokeRule exprList
                                                recog.base.set_state(114);
                                                let tmp = recog.exprList()?;
                                                if let MemberContextAll::MemberCallContext(ctx) =
                                                    cast_mut::<_, MemberContextAll>(&mut _localctx)
                                                {
                                                    ctx.args = Some(tmp.clone());
                                                } else {
                                                    unreachable!("cant cast");
                                                }
                                            }
                                        }

                                        recog.base.set_state(117);
                                        recog
                                            .base
                                            .match_token(CEL_RPAREN, &mut recog.err_handler)?;
                                    }
                                }
                                3 => {
                                    {
                                        // recRuleLabeledAltStartAction
                                        let mut tmp =
                                            IndexContextExt::new(&**MemberContextExt::new(
                                                _parentctx.clone(),
                                                _parentState,
                                            ));
                                        recog.push_new_recursion_context(
                                            tmp.clone(),
                                            _startState,
                                            RULE_member,
                                        )?;
                                        _localctx = tmp;
                                        recog.base.set_state(118);
                                        if !({
                                            let _localctx = Some(_localctx.clone());
                                            recog.precpred(None, 1)
                                        }) {
                                            Err(FailedPredicateError::new(
                                                &mut recog.base,
                                                Some("recog.precpred(None, 1)".to_owned()),
                                                None,
                                            ))?;
                                        }
                                        recog.base.set_state(119);
                                        let tmp = recog
                                            .base
                                            .match_token(CEL_LBRACKET, &mut recog.err_handler)?;
                                        if let MemberContextAll::IndexContext(ctx) =
                                            cast_mut::<_, MemberContextAll>(&mut _localctx)
                                        {
                                            ctx.op = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        recog.base.set_state(121);
                                        recog.err_handler.sync(&mut recog.base)?;
                                        _la = recog.base.input.la(1);
                                        if _la == CEL_QUESTIONMARK {
                                            {
                                                recog.base.set_state(120);
                                                let tmp = recog.base.match_token(
                                                    CEL_QUESTIONMARK,
                                                    &mut recog.err_handler,
                                                )?;
                                                if let MemberContextAll::IndexContext(ctx) =
                                                    cast_mut::<_, MemberContextAll>(&mut _localctx)
                                                {
                                                    ctx.opt = Some(tmp.clone());
                                                } else {
                                                    unreachable!("cant cast");
                                                }
                                            }
                                        }

                                        // InvokeRule expr
                                        recog.base.set_state(123);
                                        let tmp = recog.expr()?;
                                        if let MemberContextAll::IndexContext(ctx) =
                                            cast_mut::<_, MemberContextAll>(&mut _localctx)
                                        {
                                            ctx.index = Some(tmp.clone());
                                        } else {
                                            unreachable!("cant cast");
                                        }

                                        recog.base.set_state(124);
                                        recog
                                            .base
                                            .match_token(CEL_RPRACKET, &mut recog.err_handler)?;
                                    }
                                }

                                _ => {}
                            }
                        }
                    }
                    recog.base.set_state(130);
                    recog.err_handler.sync(&mut recog.base)?;
                    _alt = recog.interpreter.adaptive_predict(13, &mut recog.base)?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.unroll_recursion_context(_parentctx)?;

        Ok(_localctx)
    }
}
//------------------- primary ----------------
#[derive(Debug)]
pub enum PrimaryContextAll<'input> {
    CreateListContext(CreateListContext<'input>),
    IdentContext(IdentContext<'input>),
    CreateStructContext(CreateStructContext<'input>),
    ConstantLiteralContext(ConstantLiteralContext<'input>),
    NestedContext(NestedContext<'input>),
    CreateMessageContext(CreateMessageContext<'input>),
    GlobalCallContext(GlobalCallContext<'input>),
    Error(PrimaryContext<'input>),
}
antlr4rust::tid! {PrimaryContextAll<'a>}

impl<'input> antlr4rust::parser_rule_context::DerefSeal for PrimaryContextAll<'input> {}

impl<'input> CELParserContext<'input> for PrimaryContextAll<'input> {}

impl<'input> Deref for PrimaryContextAll<'input> {
    type Target = dyn PrimaryContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use PrimaryContextAll::*;
        match self {
            CreateListContext(inner) => inner,
            IdentContext(inner) => inner,
            CreateStructContext(inner) => inner,
            ConstantLiteralContext(inner) => inner,
            NestedContext(inner) => inner,
            CreateMessageContext(inner) => inner,
            GlobalCallContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for PrimaryContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for PrimaryContextAll<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().exit(listener)
    }
}

pub type PrimaryContext<'input> = BaseParserRuleContext<'input, PrimaryContextExt<'input>>;

#[derive(Clone)]
pub struct PrimaryContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for PrimaryContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for PrimaryContext<'input> {}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for PrimaryContext<'input> {}

impl<'input> CustomRuleContext<'input> for PrimaryContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_primary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_primary }
}
antlr4rust::tid! {PrimaryContextExt<'a>}

impl<'input> PrimaryContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<PrimaryContextAll<'input>> {
        Rc::new(PrimaryContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                PrimaryContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait PrimaryContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<PrimaryContextExt<'input>>
{
}

impl<'input> PrimaryContextAttrs<'input> for PrimaryContext<'input> {}

pub type CreateListContext<'input> = BaseParserRuleContext<'input, CreateListContextExt<'input>>;

pub trait CreateListContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token RPRACKET
    /// Returns `None` if there is no child corresponding to token RPRACKET
    fn RPRACKET(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_RPRACKET, 0)
    }
    /// Retrieves first TerminalNode corresponding to token LBRACKET
    /// Returns `None` if there is no child corresponding to token LBRACKET
    fn LBRACKET(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LBRACKET, 0)
    }
    /// Retrieves first TerminalNode corresponding to token COMMA
    /// Returns `None` if there is no child corresponding to token COMMA
    fn COMMA(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COMMA, 0)
    }
    fn listInit(&self) -> Option<Rc<ListInitContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> CreateListContextAttrs<'input> for CreateListContext<'input> {}

pub struct CreateListContextExt<'input> {
    base: PrimaryContextExt<'input>,
    pub op: Option<TokenType<'input>>,
    pub elems: Option<Rc<ListInitContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {CreateListContextExt<'a>}

impl<'input> CELParserContext<'input> for CreateListContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for CreateListContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_CreateList(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_CreateList(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for CreateListContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_CreateList(self);
    }
}

impl<'input> CustomRuleContext<'input> for CreateListContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_primary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_primary }
}

impl<'input> Borrow<PrimaryContextExt<'input>> for CreateListContext<'input> {
    fn borrow(&self) -> &PrimaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<PrimaryContextExt<'input>> for CreateListContext<'input> {
    fn borrow_mut(&mut self) -> &mut PrimaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> PrimaryContextAttrs<'input> for CreateListContext<'input> {}

impl<'input> CreateListContextExt<'input> {
    fn new(ctx: &dyn PrimaryContextAttrs<'input>) -> Rc<PrimaryContextAll<'input>> {
        Rc::new(PrimaryContextAll::CreateListContext(
            BaseParserRuleContext::copy_from(
                ctx,
                CreateListContextExt {
                    op: None,
                    elems: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type IdentContext<'input> = BaseParserRuleContext<'input, IdentContextExt<'input>>;

pub trait IdentContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token IDENTIFIER
    /// Returns `None` if there is no child corresponding to token IDENTIFIER
    fn IDENTIFIER(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_IDENTIFIER, 0)
    }
    /// Retrieves first TerminalNode corresponding to token DOT
    /// Returns `None` if there is no child corresponding to token DOT
    fn DOT(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_DOT, 0)
    }
}

impl<'input> IdentContextAttrs<'input> for IdentContext<'input> {}

pub struct IdentContextExt<'input> {
    base: PrimaryContextExt<'input>,
    pub leadingDot: Option<TokenType<'input>>,
    pub id: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {IdentContextExt<'a>}

impl<'input> CELParserContext<'input> for IdentContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for IdentContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Ident(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Ident(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for IdentContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Ident(self);
    }
}

impl<'input> CustomRuleContext<'input> for IdentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_primary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_primary }
}

impl<'input> Borrow<PrimaryContextExt<'input>> for IdentContext<'input> {
    fn borrow(&self) -> &PrimaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<PrimaryContextExt<'input>> for IdentContext<'input> {
    fn borrow_mut(&mut self) -> &mut PrimaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> PrimaryContextAttrs<'input> for IdentContext<'input> {}

impl<'input> IdentContextExt<'input> {
    fn new(ctx: &dyn PrimaryContextAttrs<'input>) -> Rc<PrimaryContextAll<'input>> {
        Rc::new(PrimaryContextAll::IdentContext(
            BaseParserRuleContext::copy_from(
                ctx,
                IdentContextExt {
                    leadingDot: None,
                    id: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type CreateStructContext<'input> =
    BaseParserRuleContext<'input, CreateStructContextExt<'input>>;

pub trait CreateStructContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token RBRACE
    /// Returns `None` if there is no child corresponding to token RBRACE
    fn RBRACE(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_RBRACE, 0)
    }
    /// Retrieves first TerminalNode corresponding to token LBRACE
    /// Returns `None` if there is no child corresponding to token LBRACE
    fn LBRACE(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LBRACE, 0)
    }
    /// Retrieves first TerminalNode corresponding to token COMMA
    /// Returns `None` if there is no child corresponding to token COMMA
    fn COMMA(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COMMA, 0)
    }
    fn mapInitializerList(&self) -> Option<Rc<MapInitializerListContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> CreateStructContextAttrs<'input> for CreateStructContext<'input> {}

pub struct CreateStructContextExt<'input> {
    base: PrimaryContextExt<'input>,
    pub op: Option<TokenType<'input>>,
    pub entries: Option<Rc<MapInitializerListContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {CreateStructContextExt<'a>}

impl<'input> CELParserContext<'input> for CreateStructContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for CreateStructContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_CreateStruct(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_CreateStruct(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for CreateStructContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_CreateStruct(self);
    }
}

impl<'input> CustomRuleContext<'input> for CreateStructContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_primary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_primary }
}

impl<'input> Borrow<PrimaryContextExt<'input>> for CreateStructContext<'input> {
    fn borrow(&self) -> &PrimaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<PrimaryContextExt<'input>> for CreateStructContext<'input> {
    fn borrow_mut(&mut self) -> &mut PrimaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> PrimaryContextAttrs<'input> for CreateStructContext<'input> {}

impl<'input> CreateStructContextExt<'input> {
    fn new(ctx: &dyn PrimaryContextAttrs<'input>) -> Rc<PrimaryContextAll<'input>> {
        Rc::new(PrimaryContextAll::CreateStructContext(
            BaseParserRuleContext::copy_from(
                ctx,
                CreateStructContextExt {
                    op: None,
                    entries: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type ConstantLiteralContext<'input> =
    BaseParserRuleContext<'input, ConstantLiteralContextExt<'input>>;

pub trait ConstantLiteralContextAttrs<'input>: CELParserContext<'input> {
    fn literal(&self) -> Option<Rc<LiteralContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> ConstantLiteralContextAttrs<'input> for ConstantLiteralContext<'input> {}

pub struct ConstantLiteralContextExt<'input> {
    base: PrimaryContextExt<'input>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {ConstantLiteralContextExt<'a>}

impl<'input> CELParserContext<'input> for ConstantLiteralContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for ConstantLiteralContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_ConstantLiteral(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_ConstantLiteral(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for ConstantLiteralContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_ConstantLiteral(self);
    }
}

impl<'input> CustomRuleContext<'input> for ConstantLiteralContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_primary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_primary }
}

impl<'input> Borrow<PrimaryContextExt<'input>> for ConstantLiteralContext<'input> {
    fn borrow(&self) -> &PrimaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<PrimaryContextExt<'input>> for ConstantLiteralContext<'input> {
    fn borrow_mut(&mut self) -> &mut PrimaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> PrimaryContextAttrs<'input> for ConstantLiteralContext<'input> {}

impl<'input> ConstantLiteralContextExt<'input> {
    fn new(ctx: &dyn PrimaryContextAttrs<'input>) -> Rc<PrimaryContextAll<'input>> {
        Rc::new(PrimaryContextAll::ConstantLiteralContext(
            BaseParserRuleContext::copy_from(
                ctx,
                ConstantLiteralContextExt {
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type NestedContext<'input> = BaseParserRuleContext<'input, NestedContextExt<'input>>;

pub trait NestedContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token LPAREN
    /// Returns `None` if there is no child corresponding to token LPAREN
    fn LPAREN(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LPAREN, 0)
    }
    /// Retrieves first TerminalNode corresponding to token RPAREN
    /// Returns `None` if there is no child corresponding to token RPAREN
    fn RPAREN(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_RPAREN, 0)
    }
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> NestedContextAttrs<'input> for NestedContext<'input> {}

pub struct NestedContextExt<'input> {
    base: PrimaryContextExt<'input>,
    pub e: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {NestedContextExt<'a>}

impl<'input> CELParserContext<'input> for NestedContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for NestedContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Nested(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Nested(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for NestedContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Nested(self);
    }
}

impl<'input> CustomRuleContext<'input> for NestedContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_primary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_primary }
}

impl<'input> Borrow<PrimaryContextExt<'input>> for NestedContext<'input> {
    fn borrow(&self) -> &PrimaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<PrimaryContextExt<'input>> for NestedContext<'input> {
    fn borrow_mut(&mut self) -> &mut PrimaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> PrimaryContextAttrs<'input> for NestedContext<'input> {}

impl<'input> NestedContextExt<'input> {
    fn new(ctx: &dyn PrimaryContextAttrs<'input>) -> Rc<PrimaryContextAll<'input>> {
        Rc::new(PrimaryContextAll::NestedContext(
            BaseParserRuleContext::copy_from(
                ctx,
                NestedContextExt {
                    e: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type CreateMessageContext<'input> =
    BaseParserRuleContext<'input, CreateMessageContextExt<'input>>;

pub trait CreateMessageContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token RBRACE
    /// Returns `None` if there is no child corresponding to token RBRACE
    fn RBRACE(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_RBRACE, 0)
    }
    /// Retrieves all `TerminalNode`s corresponding to token IDENTIFIER in current rule
    fn IDENTIFIER_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token IDENTIFIER, starting from 0.
    /// Returns `None` if number of children corresponding to token IDENTIFIER is less or equal than `i`.
    fn IDENTIFIER(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_IDENTIFIER, i)
    }
    /// Retrieves first TerminalNode corresponding to token LBRACE
    /// Returns `None` if there is no child corresponding to token LBRACE
    fn LBRACE(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LBRACE, 0)
    }
    /// Retrieves first TerminalNode corresponding to token COMMA
    /// Returns `None` if there is no child corresponding to token COMMA
    fn COMMA(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COMMA, 0)
    }
    /// Retrieves all `TerminalNode`s corresponding to token DOT in current rule
    fn DOT_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token DOT, starting from 0.
    /// Returns `None` if number of children corresponding to token DOT is less or equal than `i`.
    fn DOT(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_DOT, i)
    }
    fn field_initializer_list(&self) -> Option<Rc<Field_initializer_listContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> CreateMessageContextAttrs<'input> for CreateMessageContext<'input> {}

pub struct CreateMessageContextExt<'input> {
    base: PrimaryContextExt<'input>,
    pub leadingDot: Option<TokenType<'input>>,
    pub IDENTIFIER: Option<TokenType<'input>>,
    pub ids: Vec<TokenType<'input>>,
    pub s16: Option<TokenType<'input>>,
    pub ops: Vec<TokenType<'input>>,
    pub op: Option<TokenType<'input>>,
    pub entries: Option<Rc<Field_initializer_listContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {CreateMessageContextExt<'a>}

impl<'input> CELParserContext<'input> for CreateMessageContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for CreateMessageContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_CreateMessage(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_CreateMessage(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for CreateMessageContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_CreateMessage(self);
    }
}

impl<'input> CustomRuleContext<'input> for CreateMessageContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_primary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_primary }
}

impl<'input> Borrow<PrimaryContextExt<'input>> for CreateMessageContext<'input> {
    fn borrow(&self) -> &PrimaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<PrimaryContextExt<'input>> for CreateMessageContext<'input> {
    fn borrow_mut(&mut self) -> &mut PrimaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> PrimaryContextAttrs<'input> for CreateMessageContext<'input> {}

impl<'input> CreateMessageContextExt<'input> {
    fn new(ctx: &dyn PrimaryContextAttrs<'input>) -> Rc<PrimaryContextAll<'input>> {
        Rc::new(PrimaryContextAll::CreateMessageContext(
            BaseParserRuleContext::copy_from(
                ctx,
                CreateMessageContextExt {
                    leadingDot: None,
                    IDENTIFIER: None,
                    s16: None,
                    op: None,
                    ids: Vec::new(),
                    ops: Vec::new(),
                    entries: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type GlobalCallContext<'input> = BaseParserRuleContext<'input, GlobalCallContextExt<'input>>;

pub trait GlobalCallContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token IDENTIFIER
    /// Returns `None` if there is no child corresponding to token IDENTIFIER
    fn IDENTIFIER(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_IDENTIFIER, 0)
    }
    /// Retrieves first TerminalNode corresponding to token RPAREN
    /// Returns `None` if there is no child corresponding to token RPAREN
    fn RPAREN(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_RPAREN, 0)
    }
    /// Retrieves first TerminalNode corresponding to token LPAREN
    /// Returns `None` if there is no child corresponding to token LPAREN
    fn LPAREN(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_LPAREN, 0)
    }
    /// Retrieves first TerminalNode corresponding to token DOT
    /// Returns `None` if there is no child corresponding to token DOT
    fn DOT(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_DOT, 0)
    }
    fn exprList(&self) -> Option<Rc<ExprListContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
}

impl<'input> GlobalCallContextAttrs<'input> for GlobalCallContext<'input> {}

pub struct GlobalCallContextExt<'input> {
    base: PrimaryContextExt<'input>,
    pub leadingDot: Option<TokenType<'input>>,
    pub id: Option<TokenType<'input>>,
    pub op: Option<TokenType<'input>>,
    pub args: Option<Rc<ExprListContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {GlobalCallContextExt<'a>}

impl<'input> CELParserContext<'input> for GlobalCallContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for GlobalCallContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_GlobalCall(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_GlobalCall(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for GlobalCallContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_GlobalCall(self);
    }
}

impl<'input> CustomRuleContext<'input> for GlobalCallContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_primary
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_primary }
}

impl<'input> Borrow<PrimaryContextExt<'input>> for GlobalCallContext<'input> {
    fn borrow(&self) -> &PrimaryContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<PrimaryContextExt<'input>> for GlobalCallContext<'input> {
    fn borrow_mut(&mut self) -> &mut PrimaryContextExt<'input> {
        &mut self.base
    }
}

impl<'input> PrimaryContextAttrs<'input> for GlobalCallContext<'input> {}

impl<'input> GlobalCallContextExt<'input> {
    fn new(ctx: &dyn PrimaryContextAttrs<'input>) -> Rc<PrimaryContextAll<'input>> {
        Rc::new(PrimaryContextAll::GlobalCallContext(
            BaseParserRuleContext::copy_from(
                ctx,
                GlobalCallContextExt {
                    leadingDot: None,
                    id: None,
                    op: None,
                    args: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn primary(&mut self) -> Result<Rc<PrimaryContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = PrimaryContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 16, RULE_primary);
        let mut _localctx: Rc<PrimaryContextAll> = _localctx;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(184);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(25, &mut recog.base)? {
                1 => {
                    let tmp = IdentContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(132);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == CEL_DOT {
                            {
                                recog.base.set_state(131);
                                let tmp =
                                    recog.base.match_token(CEL_DOT, &mut recog.err_handler)?;
                                if let PrimaryContextAll::IdentContext(ctx) =
                                    cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                {
                                    ctx.leadingDot = Some(tmp.clone());
                                } else {
                                    unreachable!("cant cast");
                                }
                            }
                        }

                        recog.base.set_state(134);
                        let tmp = recog
                            .base
                            .match_token(CEL_IDENTIFIER, &mut recog.err_handler)?;
                        if let PrimaryContextAll::IdentContext(ctx) =
                            cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                        {
                            ctx.id = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                2 => {
                    let tmp = GlobalCallContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(136);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == CEL_DOT {
                            {
                                recog.base.set_state(135);
                                let tmp =
                                    recog.base.match_token(CEL_DOT, &mut recog.err_handler)?;
                                if let PrimaryContextAll::GlobalCallContext(ctx) =
                                    cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                {
                                    ctx.leadingDot = Some(tmp.clone());
                                } else {
                                    unreachable!("cant cast");
                                }
                            }
                        }

                        recog.base.set_state(138);
                        let tmp = recog
                            .base
                            .match_token(CEL_IDENTIFIER, &mut recog.err_handler)?;
                        if let PrimaryContextAll::GlobalCallContext(ctx) =
                            cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                        {
                            ctx.id = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        {
                            recog.base.set_state(139);
                            let tmp = recog.base.match_token(CEL_LPAREN, &mut recog.err_handler)?;
                            if let PrimaryContextAll::GlobalCallContext(ctx) =
                                cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                            {
                                ctx.op = Some(tmp.clone());
                            } else {
                                unreachable!("cant cast");
                            }

                            recog.base.set_state(141);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                            if ((_la - 10) & !0x3f) == 0
                                && ((1usize << (_la - 10)) & 132580181) != 0
                            {
                                {
                                    // InvokeRule exprList
                                    recog.base.set_state(140);
                                    let tmp = recog.exprList()?;
                                    if let PrimaryContextAll::GlobalCallContext(ctx) =
                                        cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                    {
                                        ctx.args = Some(tmp.clone());
                                    } else {
                                        unreachable!("cant cast");
                                    }
                                }
                            }

                            recog.base.set_state(143);
                            recog.base.match_token(CEL_RPAREN, &mut recog.err_handler)?;
                        }
                    }
                }
                3 => {
                    let tmp = NestedContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(144);
                        recog.base.match_token(CEL_LPAREN, &mut recog.err_handler)?;

                        // InvokeRule expr
                        recog.base.set_state(145);
                        let tmp = recog.expr()?;
                        if let PrimaryContextAll::NestedContext(ctx) =
                            cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                        {
                            ctx.e = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        recog.base.set_state(146);
                        recog.base.match_token(CEL_RPAREN, &mut recog.err_handler)?;
                    }
                }
                4 => {
                    let tmp = CreateListContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 4)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(148);
                        let tmp = recog
                            .base
                            .match_token(CEL_LBRACKET, &mut recog.err_handler)?;
                        if let PrimaryContextAll::CreateListContext(ctx) =
                            cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                        {
                            ctx.op = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        recog.base.set_state(150);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if ((_la - 10) & !0x3f) == 0 && ((1usize << (_la - 10)) & 132581205) != 0 {
                            {
                                // InvokeRule listInit
                                recog.base.set_state(149);
                                let tmp = recog.listInit()?;
                                if let PrimaryContextAll::CreateListContext(ctx) =
                                    cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                {
                                    ctx.elems = Some(tmp.clone());
                                } else {
                                    unreachable!("cant cast");
                                }
                            }
                        }

                        recog.base.set_state(153);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == CEL_COMMA {
                            {
                                recog.base.set_state(152);
                                recog.base.match_token(CEL_COMMA, &mut recog.err_handler)?;
                            }
                        }

                        recog.base.set_state(155);
                        recog
                            .base
                            .match_token(CEL_RPRACKET, &mut recog.err_handler)?;
                    }
                }
                5 => {
                    let tmp = CreateStructContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 5)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(156);
                        let tmp = recog.base.match_token(CEL_LBRACE, &mut recog.err_handler)?;
                        if let PrimaryContextAll::CreateStructContext(ctx) =
                            cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                        {
                            ctx.op = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        recog.base.set_state(158);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if ((_la - 10) & !0x3f) == 0 && ((1usize << (_la - 10)) & 132581205) != 0 {
                            {
                                // InvokeRule mapInitializerList
                                recog.base.set_state(157);
                                let tmp = recog.mapInitializerList()?;
                                if let PrimaryContextAll::CreateStructContext(ctx) =
                                    cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                {
                                    ctx.entries = Some(tmp.clone());
                                } else {
                                    unreachable!("cant cast");
                                }
                            }
                        }

                        recog.base.set_state(161);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == CEL_COMMA {
                            {
                                recog.base.set_state(160);
                                recog.base.match_token(CEL_COMMA, &mut recog.err_handler)?;
                            }
                        }

                        recog.base.set_state(163);
                        recog.base.match_token(CEL_RBRACE, &mut recog.err_handler)?;
                    }
                }
                6 => {
                    let tmp = CreateMessageContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 6)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(165);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == CEL_DOT {
                            {
                                recog.base.set_state(164);
                                let tmp =
                                    recog.base.match_token(CEL_DOT, &mut recog.err_handler)?;
                                if let PrimaryContextAll::CreateMessageContext(ctx) =
                                    cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                {
                                    ctx.leadingDot = Some(tmp.clone());
                                } else {
                                    unreachable!("cant cast");
                                }
                            }
                        }

                        recog.base.set_state(167);
                        let tmp = recog
                            .base
                            .match_token(CEL_IDENTIFIER, &mut recog.err_handler)?;
                        if let PrimaryContextAll::CreateMessageContext(ctx) =
                            cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                        {
                            ctx.IDENTIFIER = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        let temp = if let PrimaryContextAll::CreateMessageContext(ctx) =
                            cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                        {
                            ctx.IDENTIFIER.clone().unwrap()
                        } else {
                            unreachable!("cant cast");
                        };
                        if let PrimaryContextAll::CreateMessageContext(ctx) =
                            cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                        {
                            ctx.ids.push(temp);
                        } else {
                            unreachable!("cant cast");
                        }
                        recog.base.set_state(172);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        while _la == CEL_DOT {
                            {
                                {
                                    recog.base.set_state(168);
                                    let tmp =
                                        recog.base.match_token(CEL_DOT, &mut recog.err_handler)?;
                                    if let PrimaryContextAll::CreateMessageContext(ctx) =
                                        cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                    {
                                        ctx.s16 = Some(tmp.clone());
                                    } else {
                                        unreachable!("cant cast");
                                    }

                                    let temp = if let PrimaryContextAll::CreateMessageContext(ctx) =
                                        cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                    {
                                        ctx.s16.clone().unwrap()
                                    } else {
                                        unreachable!("cant cast");
                                    };
                                    if let PrimaryContextAll::CreateMessageContext(ctx) =
                                        cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                    {
                                        ctx.ops.push(temp);
                                    } else {
                                        unreachable!("cant cast");
                                    }
                                    recog.base.set_state(169);
                                    let tmp = recog
                                        .base
                                        .match_token(CEL_IDENTIFIER, &mut recog.err_handler)?;
                                    if let PrimaryContextAll::CreateMessageContext(ctx) =
                                        cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                    {
                                        ctx.IDENTIFIER = Some(tmp.clone());
                                    } else {
                                        unreachable!("cant cast");
                                    }

                                    let temp = if let PrimaryContextAll::CreateMessageContext(ctx) =
                                        cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                    {
                                        ctx.IDENTIFIER.clone().unwrap()
                                    } else {
                                        unreachable!("cant cast");
                                    };
                                    if let PrimaryContextAll::CreateMessageContext(ctx) =
                                        cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                    {
                                        ctx.ids.push(temp);
                                    } else {
                                        unreachable!("cant cast");
                                    }
                                }
                            }
                            recog.base.set_state(174);
                            recog.err_handler.sync(&mut recog.base)?;
                            _la = recog.base.input.la(1);
                        }
                        recog.base.set_state(175);
                        let tmp = recog.base.match_token(CEL_LBRACE, &mut recog.err_handler)?;
                        if let PrimaryContextAll::CreateMessageContext(ctx) =
                            cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                        {
                            ctx.op = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }

                        recog.base.set_state(177);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if ((_la - 20) & !0x3f) == 0 && ((1usize << (_la - 20)) & 196609) != 0 {
                            {
                                // InvokeRule field_initializer_list
                                recog.base.set_state(176);
                                let tmp = recog.field_initializer_list()?;
                                if let PrimaryContextAll::CreateMessageContext(ctx) =
                                    cast_mut::<_, PrimaryContextAll>(&mut _localctx)
                                {
                                    ctx.entries = Some(tmp.clone());
                                } else {
                                    unreachable!("cant cast");
                                }
                            }
                        }

                        recog.base.set_state(180);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == CEL_COMMA {
                            {
                                recog.base.set_state(179);
                                recog.base.match_token(CEL_COMMA, &mut recog.err_handler)?;
                            }
                        }

                        recog.base.set_state(182);
                        recog.base.match_token(CEL_RBRACE, &mut recog.err_handler)?;
                    }
                }
                7 => {
                    let tmp = ConstantLiteralContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 7)?;
                    _localctx = tmp;
                    {
                        // InvokeRule literal
                        recog.base.set_state(183);
                        recog.literal()?;
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- exprList ----------------
pub type ExprListContextAll<'input> = ExprListContext<'input>;

pub type ExprListContext<'input> = BaseParserRuleContext<'input, ExprListContextExt<'input>>;

#[derive(Clone)]
pub struct ExprListContextExt<'input> {
    pub expr: Option<Rc<ExprContextAll<'input>>>,
    pub e: Vec<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for ExprListContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for ExprListContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_exprList(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_exprList(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for ExprListContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_exprList(self);
    }
}

impl<'input> CustomRuleContext<'input> for ExprListContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_exprList
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_exprList }
}
antlr4rust::tid! {ExprListContextExt<'a>}

impl<'input> ExprListContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<ExprListContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            ExprListContextExt {
                expr: None,
                e: Vec::new(),

                ph: PhantomData,
            },
        ))
    }
}

pub trait ExprListContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<ExprListContextExt<'input>>
{
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves all `TerminalNode`s corresponding to token COMMA in current rule
    fn COMMA_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token COMMA, starting from 0.
    /// Returns `None` if number of children corresponding to token COMMA is less or equal than `i`.
    fn COMMA(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COMMA, i)
    }
}

impl<'input> ExprListContextAttrs<'input> for ExprListContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn exprList(&mut self) -> Result<Rc<ExprListContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = ExprListContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 18, RULE_exprList);
        let mut _localctx: Rc<ExprListContextAll> = _localctx;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                // InvokeRule expr
                recog.base.set_state(186);
                let tmp = recog.expr()?;
                cast_mut::<_, ExprListContext>(&mut _localctx).expr = Some(tmp.clone());

                let temp = cast_mut::<_, ExprListContext>(&mut _localctx)
                    .expr
                    .clone()
                    .unwrap();
                cast_mut::<_, ExprListContext>(&mut _localctx).e.push(temp);

                recog.base.set_state(191);
                recog.err_handler.sync(&mut recog.base)?;
                _la = recog.base.input.la(1);
                while _la == CEL_COMMA {
                    {
                        {
                            recog.base.set_state(187);
                            recog.base.match_token(CEL_COMMA, &mut recog.err_handler)?;

                            // InvokeRule expr
                            recog.base.set_state(188);
                            let tmp = recog.expr()?;
                            cast_mut::<_, ExprListContext>(&mut _localctx).expr = Some(tmp.clone());

                            let temp = cast_mut::<_, ExprListContext>(&mut _localctx)
                                .expr
                                .clone()
                                .unwrap();
                            cast_mut::<_, ExprListContext>(&mut _localctx).e.push(temp);
                        }
                    }
                    recog.base.set_state(193);
                    recog.err_handler.sync(&mut recog.base)?;
                    _la = recog.base.input.la(1);
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- listInit ----------------
pub type ListInitContextAll<'input> = ListInitContext<'input>;

pub type ListInitContext<'input> = BaseParserRuleContext<'input, ListInitContextExt<'input>>;

#[derive(Clone)]
pub struct ListInitContextExt<'input> {
    pub optExpr: Option<Rc<OptExprContextAll<'input>>>,
    pub elems: Vec<Rc<OptExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for ListInitContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for ListInitContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_listInit(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_listInit(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for ListInitContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_listInit(self);
    }
}

impl<'input> CustomRuleContext<'input> for ListInitContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_listInit
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_listInit }
}
antlr4rust::tid! {ListInitContextExt<'a>}

impl<'input> ListInitContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<ListInitContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            ListInitContextExt {
                optExpr: None,
                elems: Vec::new(),

                ph: PhantomData,
            },
        ))
    }
}

pub trait ListInitContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<ListInitContextExt<'input>>
{
    fn optExpr_all(&self) -> Vec<Rc<OptExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn optExpr(&self, i: usize) -> Option<Rc<OptExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves all `TerminalNode`s corresponding to token COMMA in current rule
    fn COMMA_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token COMMA, starting from 0.
    /// Returns `None` if number of children corresponding to token COMMA is less or equal than `i`.
    fn COMMA(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COMMA, i)
    }
}

impl<'input> ListInitContextAttrs<'input> for ListInitContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn listInit(&mut self) -> Result<Rc<ListInitContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = ListInitContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 20, RULE_listInit);
        let mut _localctx: Rc<ListInitContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: i32;
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                // InvokeRule optExpr
                recog.base.set_state(194);
                let tmp = recog.optExpr()?;
                cast_mut::<_, ListInitContext>(&mut _localctx).optExpr = Some(tmp.clone());

                let temp = cast_mut::<_, ListInitContext>(&mut _localctx)
                    .optExpr
                    .clone()
                    .unwrap();
                cast_mut::<_, ListInitContext>(&mut _localctx)
                    .elems
                    .push(temp);

                recog.base.set_state(199);
                recog.err_handler.sync(&mut recog.base)?;
                _alt = recog.interpreter.adaptive_predict(27, &mut recog.base)?;
                while { _alt != 2 && _alt != INVALID_ALT } {
                    if _alt == 1 {
                        {
                            {
                                recog.base.set_state(195);
                                recog.base.match_token(CEL_COMMA, &mut recog.err_handler)?;

                                // InvokeRule optExpr
                                recog.base.set_state(196);
                                let tmp = recog.optExpr()?;
                                cast_mut::<_, ListInitContext>(&mut _localctx).optExpr =
                                    Some(tmp.clone());

                                let temp = cast_mut::<_, ListInitContext>(&mut _localctx)
                                    .optExpr
                                    .clone()
                                    .unwrap();
                                cast_mut::<_, ListInitContext>(&mut _localctx)
                                    .elems
                                    .push(temp);
                            }
                        }
                    }
                    recog.base.set_state(201);
                    recog.err_handler.sync(&mut recog.base)?;
                    _alt = recog.interpreter.adaptive_predict(27, &mut recog.base)?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- field_initializer_list ----------------
pub type Field_initializer_listContextAll<'input> = Field_initializer_listContext<'input>;

pub type Field_initializer_listContext<'input> =
    BaseParserRuleContext<'input, Field_initializer_listContextExt<'input>>;

#[derive(Clone)]
pub struct Field_initializer_listContextExt<'input> {
    pub optField: Option<Rc<OptFieldContextAll<'input>>>,
    pub fields: Vec<Rc<OptFieldContextAll<'input>>>,
    pub s21: Option<TokenType<'input>>,
    pub cols: Vec<TokenType<'input>>,
    pub expr: Option<Rc<ExprContextAll<'input>>>,
    pub values: Vec<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for Field_initializer_listContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a>
    for Field_initializer_listContext<'input>
{
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_field_initializer_list(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_field_initializer_list(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for Field_initializer_listContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_field_initializer_list(self);
    }
}

impl<'input> CustomRuleContext<'input> for Field_initializer_listContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_field_initializer_list
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_field_initializer_list }
}
antlr4rust::tid! {Field_initializer_listContextExt<'a>}

impl<'input> Field_initializer_listContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<Field_initializer_listContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            Field_initializer_listContextExt {
                s21: None,
                cols: Vec::new(),
                optField: None,
                expr: None,
                fields: Vec::new(),
                values: Vec::new(),

                ph: PhantomData,
            },
        ))
    }
}

pub trait Field_initializer_listContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<Field_initializer_listContextExt<'input>>
{
    fn optField_all(&self) -> Vec<Rc<OptFieldContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn optField(&self, i: usize) -> Option<Rc<OptFieldContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves all `TerminalNode`s corresponding to token COLON in current rule
    fn COLON_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token COLON, starting from 0.
    /// Returns `None` if number of children corresponding to token COLON is less or equal than `i`.
    fn COLON(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COLON, i)
    }
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves all `TerminalNode`s corresponding to token COMMA in current rule
    fn COMMA_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token COMMA, starting from 0.
    /// Returns `None` if number of children corresponding to token COMMA is less or equal than `i`.
    fn COMMA(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COMMA, i)
    }
}

impl<'input> Field_initializer_listContextAttrs<'input> for Field_initializer_listContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn field_initializer_list(
        &mut self,
    ) -> Result<Rc<Field_initializer_listContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx =
            Field_initializer_listContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_rule(_localctx.clone(), 22, RULE_field_initializer_list);
        let mut _localctx: Rc<Field_initializer_listContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: i32;
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                // InvokeRule optField
                recog.base.set_state(202);
                let tmp = recog.optField()?;
                cast_mut::<_, Field_initializer_listContext>(&mut _localctx).optField =
                    Some(tmp.clone());

                let temp = cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                    .optField
                    .clone()
                    .unwrap();
                cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                    .fields
                    .push(temp);

                recog.base.set_state(203);
                let tmp = recog.base.match_token(CEL_COLON, &mut recog.err_handler)?;
                cast_mut::<_, Field_initializer_listContext>(&mut _localctx).s21 =
                    Some(tmp.clone());

                let temp = cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                    .s21
                    .clone()
                    .unwrap();
                cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                    .cols
                    .push(temp);

                // InvokeRule expr
                recog.base.set_state(204);
                let tmp = recog.expr()?;
                cast_mut::<_, Field_initializer_listContext>(&mut _localctx).expr =
                    Some(tmp.clone());

                let temp = cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                    .expr
                    .clone()
                    .unwrap();
                cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                    .values
                    .push(temp);

                recog.base.set_state(212);
                recog.err_handler.sync(&mut recog.base)?;
                _alt = recog.interpreter.adaptive_predict(28, &mut recog.base)?;
                while { _alt != 2 && _alt != INVALID_ALT } {
                    if _alt == 1 {
                        {
                            {
                                recog.base.set_state(205);
                                recog.base.match_token(CEL_COMMA, &mut recog.err_handler)?;

                                // InvokeRule optField
                                recog.base.set_state(206);
                                let tmp = recog.optField()?;
                                cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                                    .optField = Some(tmp.clone());

                                let temp =
                                    cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                                        .optField
                                        .clone()
                                        .unwrap();
                                cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                                    .fields
                                    .push(temp);

                                recog.base.set_state(207);
                                let tmp =
                                    recog.base.match_token(CEL_COLON, &mut recog.err_handler)?;
                                cast_mut::<_, Field_initializer_listContext>(&mut _localctx).s21 =
                                    Some(tmp.clone());

                                let temp =
                                    cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                                        .s21
                                        .clone()
                                        .unwrap();
                                cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                                    .cols
                                    .push(temp);

                                // InvokeRule expr
                                recog.base.set_state(208);
                                let tmp = recog.expr()?;
                                cast_mut::<_, Field_initializer_listContext>(&mut _localctx).expr =
                                    Some(tmp.clone());

                                let temp =
                                    cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                                        .expr
                                        .clone()
                                        .unwrap();
                                cast_mut::<_, Field_initializer_listContext>(&mut _localctx)
                                    .values
                                    .push(temp);
                            }
                        }
                    }
                    recog.base.set_state(214);
                    recog.err_handler.sync(&mut recog.base)?;
                    _alt = recog.interpreter.adaptive_predict(28, &mut recog.base)?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- optField ----------------
pub type OptFieldContextAll<'input> = OptFieldContext<'input>;

pub type OptFieldContext<'input> = BaseParserRuleContext<'input, OptFieldContextExt<'input>>;

#[derive(Clone)]
pub struct OptFieldContextExt<'input> {
    pub opt: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for OptFieldContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for OptFieldContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_optField(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_optField(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for OptFieldContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_optField(self);
    }
}

impl<'input> CustomRuleContext<'input> for OptFieldContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_optField
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_optField }
}
antlr4rust::tid! {OptFieldContextExt<'a>}

impl<'input> OptFieldContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<OptFieldContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            OptFieldContextExt {
                opt: None,

                ph: PhantomData,
            },
        ))
    }
}

pub trait OptFieldContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<OptFieldContextExt<'input>>
{
    fn escapeIdent(&self) -> Option<Rc<EscapeIdentContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    /// Retrieves first TerminalNode corresponding to token QUESTIONMARK
    /// Returns `None` if there is no child corresponding to token QUESTIONMARK
    fn QUESTIONMARK(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_QUESTIONMARK, 0)
    }
}

impl<'input> OptFieldContextAttrs<'input> for OptFieldContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn optField(&mut self) -> Result<Rc<OptFieldContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = OptFieldContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 24, RULE_optField);
        let mut _localctx: Rc<OptFieldContextAll> = _localctx;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                recog.base.set_state(216);
                recog.err_handler.sync(&mut recog.base)?;
                _la = recog.base.input.la(1);
                if _la == CEL_QUESTIONMARK {
                    {
                        recog.base.set_state(215);
                        let tmp = recog
                            .base
                            .match_token(CEL_QUESTIONMARK, &mut recog.err_handler)?;
                        cast_mut::<_, OptFieldContext>(&mut _localctx).opt = Some(tmp.clone());
                    }
                }

                // InvokeRule escapeIdent
                recog.base.set_state(218);
                recog.escapeIdent()?;
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- mapInitializerList ----------------
pub type MapInitializerListContextAll<'input> = MapInitializerListContext<'input>;

pub type MapInitializerListContext<'input> =
    BaseParserRuleContext<'input, MapInitializerListContextExt<'input>>;

#[derive(Clone)]
pub struct MapInitializerListContextExt<'input> {
    pub optExpr: Option<Rc<OptExprContextAll<'input>>>,
    pub keys: Vec<Rc<OptExprContextAll<'input>>>,
    pub s21: Option<TokenType<'input>>,
    pub cols: Vec<TokenType<'input>>,
    pub expr: Option<Rc<ExprContextAll<'input>>>,
    pub values: Vec<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for MapInitializerListContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for MapInitializerListContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_mapInitializerList(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_mapInitializerList(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for MapInitializerListContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_mapInitializerList(self);
    }
}

impl<'input> CustomRuleContext<'input> for MapInitializerListContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_mapInitializerList
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_mapInitializerList }
}
antlr4rust::tid! {MapInitializerListContextExt<'a>}

impl<'input> MapInitializerListContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<MapInitializerListContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            MapInitializerListContextExt {
                s21: None,
                cols: Vec::new(),
                optExpr: None,
                expr: None,
                keys: Vec::new(),
                values: Vec::new(),

                ph: PhantomData,
            },
        ))
    }
}

pub trait MapInitializerListContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<MapInitializerListContextExt<'input>>
{
    fn optExpr_all(&self) -> Vec<Rc<OptExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn optExpr(&self, i: usize) -> Option<Rc<OptExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves all `TerminalNode`s corresponding to token COLON in current rule
    fn COLON_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token COLON, starting from 0.
    /// Returns `None` if number of children corresponding to token COLON is less or equal than `i`.
    fn COLON(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COLON, i)
    }
    fn expr_all(&self) -> Vec<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    fn expr(&self, i: usize) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(i)
    }
    /// Retrieves all `TerminalNode`s corresponding to token COMMA in current rule
    fn COMMA_all(&self) -> Vec<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.children_of_type()
    }
    /// Retrieves 'i's TerminalNode corresponding to token COMMA, starting from 0.
    /// Returns `None` if number of children corresponding to token COMMA is less or equal than `i`.
    fn COMMA(&self, i: usize) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_COMMA, i)
    }
}

impl<'input> MapInitializerListContextAttrs<'input> for MapInitializerListContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn mapInitializerList(
        &mut self,
    ) -> Result<Rc<MapInitializerListContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx =
            MapInitializerListContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_rule(_localctx.clone(), 26, RULE_mapInitializerList);
        let mut _localctx: Rc<MapInitializerListContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            let mut _alt: i32;
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                // InvokeRule optExpr
                recog.base.set_state(220);
                let tmp = recog.optExpr()?;
                cast_mut::<_, MapInitializerListContext>(&mut _localctx).optExpr =
                    Some(tmp.clone());

                let temp = cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                    .optExpr
                    .clone()
                    .unwrap();
                cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                    .keys
                    .push(temp);

                recog.base.set_state(221);
                let tmp = recog.base.match_token(CEL_COLON, &mut recog.err_handler)?;
                cast_mut::<_, MapInitializerListContext>(&mut _localctx).s21 = Some(tmp.clone());

                let temp = cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                    .s21
                    .clone()
                    .unwrap();
                cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                    .cols
                    .push(temp);

                // InvokeRule expr
                recog.base.set_state(222);
                let tmp = recog.expr()?;
                cast_mut::<_, MapInitializerListContext>(&mut _localctx).expr = Some(tmp.clone());

                let temp = cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                    .expr
                    .clone()
                    .unwrap();
                cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                    .values
                    .push(temp);

                recog.base.set_state(230);
                recog.err_handler.sync(&mut recog.base)?;
                _alt = recog.interpreter.adaptive_predict(30, &mut recog.base)?;
                while { _alt != 2 && _alt != INVALID_ALT } {
                    if _alt == 1 {
                        {
                            {
                                recog.base.set_state(223);
                                recog.base.match_token(CEL_COMMA, &mut recog.err_handler)?;

                                // InvokeRule optExpr
                                recog.base.set_state(224);
                                let tmp = recog.optExpr()?;
                                cast_mut::<_, MapInitializerListContext>(&mut _localctx).optExpr =
                                    Some(tmp.clone());

                                let temp = cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                                    .optExpr
                                    .clone()
                                    .unwrap();
                                cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                                    .keys
                                    .push(temp);

                                recog.base.set_state(225);
                                let tmp =
                                    recog.base.match_token(CEL_COLON, &mut recog.err_handler)?;
                                cast_mut::<_, MapInitializerListContext>(&mut _localctx).s21 =
                                    Some(tmp.clone());

                                let temp = cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                                    .s21
                                    .clone()
                                    .unwrap();
                                cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                                    .cols
                                    .push(temp);

                                // InvokeRule expr
                                recog.base.set_state(226);
                                let tmp = recog.expr()?;
                                cast_mut::<_, MapInitializerListContext>(&mut _localctx).expr =
                                    Some(tmp.clone());

                                let temp = cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                                    .expr
                                    .clone()
                                    .unwrap();
                                cast_mut::<_, MapInitializerListContext>(&mut _localctx)
                                    .values
                                    .push(temp);
                            }
                        }
                    }
                    recog.base.set_state(232);
                    recog.err_handler.sync(&mut recog.base)?;
                    _alt = recog.interpreter.adaptive_predict(30, &mut recog.base)?;
                }
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- escapeIdent ----------------
#[derive(Debug)]
pub enum EscapeIdentContextAll<'input> {
    EscapedIdentifierContext(EscapedIdentifierContext<'input>),
    SimpleIdentifierContext(SimpleIdentifierContext<'input>),
    Error(EscapeIdentContext<'input>),
}
antlr4rust::tid! {EscapeIdentContextAll<'a>}

impl<'input> antlr4rust::parser_rule_context::DerefSeal for EscapeIdentContextAll<'input> {}

impl<'input> CELParserContext<'input> for EscapeIdentContextAll<'input> {}

impl<'input> Deref for EscapeIdentContextAll<'input> {
    type Target = dyn EscapeIdentContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use EscapeIdentContextAll::*;
        match self {
            EscapedIdentifierContext(inner) => inner,
            SimpleIdentifierContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for EscapeIdentContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for EscapeIdentContextAll<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().exit(listener)
    }
}

pub type EscapeIdentContext<'input> = BaseParserRuleContext<'input, EscapeIdentContextExt<'input>>;

#[derive(Clone)]
pub struct EscapeIdentContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for EscapeIdentContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for EscapeIdentContext<'input> {}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for EscapeIdentContext<'input> {}

impl<'input> CustomRuleContext<'input> for EscapeIdentContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_escapeIdent
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_escapeIdent }
}
antlr4rust::tid! {EscapeIdentContextExt<'a>}

impl<'input> EscapeIdentContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<EscapeIdentContextAll<'input>> {
        Rc::new(EscapeIdentContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                EscapeIdentContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait EscapeIdentContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<EscapeIdentContextExt<'input>>
{
}

impl<'input> EscapeIdentContextAttrs<'input> for EscapeIdentContext<'input> {}

pub type EscapedIdentifierContext<'input> =
    BaseParserRuleContext<'input, EscapedIdentifierContextExt<'input>>;

pub trait EscapedIdentifierContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token ESC_IDENTIFIER
    /// Returns `None` if there is no child corresponding to token ESC_IDENTIFIER
    fn ESC_IDENTIFIER(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_ESC_IDENTIFIER, 0)
    }
}

impl<'input> EscapedIdentifierContextAttrs<'input> for EscapedIdentifierContext<'input> {}

pub struct EscapedIdentifierContextExt<'input> {
    base: EscapeIdentContextExt<'input>,
    pub id: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {EscapedIdentifierContextExt<'a>}

impl<'input> CELParserContext<'input> for EscapedIdentifierContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for EscapedIdentifierContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_EscapedIdentifier(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_EscapedIdentifier(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for EscapedIdentifierContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_EscapedIdentifier(self);
    }
}

impl<'input> CustomRuleContext<'input> for EscapedIdentifierContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_escapeIdent
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_escapeIdent }
}

impl<'input> Borrow<EscapeIdentContextExt<'input>> for EscapedIdentifierContext<'input> {
    fn borrow(&self) -> &EscapeIdentContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<EscapeIdentContextExt<'input>> for EscapedIdentifierContext<'input> {
    fn borrow_mut(&mut self) -> &mut EscapeIdentContextExt<'input> {
        &mut self.base
    }
}

impl<'input> EscapeIdentContextAttrs<'input> for EscapedIdentifierContext<'input> {}

impl<'input> EscapedIdentifierContextExt<'input> {
    fn new(ctx: &dyn EscapeIdentContextAttrs<'input>) -> Rc<EscapeIdentContextAll<'input>> {
        Rc::new(EscapeIdentContextAll::EscapedIdentifierContext(
            BaseParserRuleContext::copy_from(
                ctx,
                EscapedIdentifierContextExt {
                    id: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type SimpleIdentifierContext<'input> =
    BaseParserRuleContext<'input, SimpleIdentifierContextExt<'input>>;

pub trait SimpleIdentifierContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token IDENTIFIER
    /// Returns `None` if there is no child corresponding to token IDENTIFIER
    fn IDENTIFIER(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_IDENTIFIER, 0)
    }
}

impl<'input> SimpleIdentifierContextAttrs<'input> for SimpleIdentifierContext<'input> {}

pub struct SimpleIdentifierContextExt<'input> {
    base: EscapeIdentContextExt<'input>,
    pub id: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {SimpleIdentifierContextExt<'a>}

impl<'input> CELParserContext<'input> for SimpleIdentifierContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for SimpleIdentifierContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_SimpleIdentifier(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_SimpleIdentifier(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for SimpleIdentifierContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_SimpleIdentifier(self);
    }
}

impl<'input> CustomRuleContext<'input> for SimpleIdentifierContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_escapeIdent
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_escapeIdent }
}

impl<'input> Borrow<EscapeIdentContextExt<'input>> for SimpleIdentifierContext<'input> {
    fn borrow(&self) -> &EscapeIdentContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<EscapeIdentContextExt<'input>> for SimpleIdentifierContext<'input> {
    fn borrow_mut(&mut self) -> &mut EscapeIdentContextExt<'input> {
        &mut self.base
    }
}

impl<'input> EscapeIdentContextAttrs<'input> for SimpleIdentifierContext<'input> {}

impl<'input> SimpleIdentifierContextExt<'input> {
    fn new(ctx: &dyn EscapeIdentContextAttrs<'input>) -> Rc<EscapeIdentContextAll<'input>> {
        Rc::new(EscapeIdentContextAll::SimpleIdentifierContext(
            BaseParserRuleContext::copy_from(
                ctx,
                SimpleIdentifierContextExt {
                    id: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn escapeIdent(&mut self) -> Result<Rc<EscapeIdentContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = EscapeIdentContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog
            .base
            .enter_rule(_localctx.clone(), 28, RULE_escapeIdent);
        let mut _localctx: Rc<EscapeIdentContextAll> = _localctx;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(235);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.base.input.la(1) {
                CEL_IDENTIFIER => {
                    let tmp = SimpleIdentifierContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(233);
                        let tmp = recog
                            .base
                            .match_token(CEL_IDENTIFIER, &mut recog.err_handler)?;
                        if let EscapeIdentContextAll::SimpleIdentifierContext(ctx) =
                            cast_mut::<_, EscapeIdentContextAll>(&mut _localctx)
                        {
                            ctx.id = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }

                CEL_ESC_IDENTIFIER => {
                    let tmp = EscapedIdentifierContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(234);
                        let tmp = recog
                            .base
                            .match_token(CEL_ESC_IDENTIFIER, &mut recog.err_handler)?;
                        if let EscapeIdentContextAll::EscapedIdentifierContext(ctx) =
                            cast_mut::<_, EscapeIdentContextAll>(&mut _localctx)
                        {
                            ctx.id = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }

                _ => Err(ANTLRError::NoAltError(NoViableAltError::new(
                    &mut recog.base,
                )))?,
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- optExpr ----------------
pub type OptExprContextAll<'input> = OptExprContext<'input>;

pub type OptExprContext<'input> = BaseParserRuleContext<'input, OptExprContextExt<'input>>;

#[derive(Clone)]
pub struct OptExprContextExt<'input> {
    pub opt: Option<TokenType<'input>>,
    pub e: Option<Rc<ExprContextAll<'input>>>,
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for OptExprContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for OptExprContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_optExpr(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_optExpr(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for OptExprContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_optExpr(self);
    }
}

impl<'input> CustomRuleContext<'input> for OptExprContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_optExpr
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_optExpr }
}
antlr4rust::tid! {OptExprContextExt<'a>}

impl<'input> OptExprContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<OptExprContextAll<'input>> {
        Rc::new(BaseParserRuleContext::new_parser_ctx(
            parent,
            invoking_state,
            OptExprContextExt {
                opt: None,
                e: None,

                ph: PhantomData,
            },
        ))
    }
}

pub trait OptExprContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<OptExprContextExt<'input>>
{
    fn expr(&self) -> Option<Rc<ExprContextAll<'input>>>
    where
        Self: Sized,
    {
        self.child_of_type(0)
    }
    /// Retrieves first TerminalNode corresponding to token QUESTIONMARK
    /// Returns `None` if there is no child corresponding to token QUESTIONMARK
    fn QUESTIONMARK(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_QUESTIONMARK, 0)
    }
}

impl<'input> OptExprContextAttrs<'input> for OptExprContext<'input> {}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn optExpr(&mut self) -> Result<Rc<OptExprContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = OptExprContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 30, RULE_optExpr);
        let mut _localctx: Rc<OptExprContextAll> = _localctx;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            // recog.base.enter_outer_alt(_localctx.clone(), 1)?;
            recog.base.enter_outer_alt(None, 1)?;
            {
                recog.base.set_state(238);
                recog.err_handler.sync(&mut recog.base)?;
                _la = recog.base.input.la(1);
                if _la == CEL_QUESTIONMARK {
                    {
                        recog.base.set_state(237);
                        let tmp = recog
                            .base
                            .match_token(CEL_QUESTIONMARK, &mut recog.err_handler)?;
                        cast_mut::<_, OptExprContext>(&mut _localctx).opt = Some(tmp.clone());
                    }
                }

                // InvokeRule expr
                recog.base.set_state(240);
                let tmp = recog.expr()?;
                cast_mut::<_, OptExprContext>(&mut _localctx).e = Some(tmp.clone());
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
//------------------- literal ----------------
#[derive(Debug)]
pub enum LiteralContextAll<'input> {
    BytesContext(BytesContext<'input>),
    UintContext(UintContext<'input>),
    NullContext(NullContext<'input>),
    BoolFalseContext(BoolFalseContext<'input>),
    StringContext(StringContext<'input>),
    DoubleContext(DoubleContext<'input>),
    BoolTrueContext(BoolTrueContext<'input>),
    IntContext(IntContext<'input>),
    Error(LiteralContext<'input>),
}
antlr4rust::tid! {LiteralContextAll<'a>}

impl<'input> antlr4rust::parser_rule_context::DerefSeal for LiteralContextAll<'input> {}

impl<'input> CELParserContext<'input> for LiteralContextAll<'input> {}

impl<'input> Deref for LiteralContextAll<'input> {
    type Target = dyn LiteralContextAttrs<'input> + 'input;
    fn deref(&self) -> &Self::Target {
        use LiteralContextAll::*;
        match self {
            BytesContext(inner) => inner,
            UintContext(inner) => inner,
            NullContext(inner) => inner,
            BoolFalseContext(inner) => inner,
            StringContext(inner) => inner,
            DoubleContext(inner) => inner,
            BoolTrueContext(inner) => inner,
            IntContext(inner) => inner,
            Error(inner) => inner,
        }
    }
}
impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for LiteralContextAll<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        self.deref().accept(visitor)
    }
}
impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for LiteralContextAll<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().enter(listener)
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        self.deref().exit(listener)
    }
}

pub type LiteralContext<'input> = BaseParserRuleContext<'input, LiteralContextExt<'input>>;

#[derive(Clone)]
pub struct LiteralContextExt<'input> {
    ph: PhantomData<&'input str>,
}

impl<'input> CELParserContext<'input> for LiteralContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for LiteralContext<'input> {}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for LiteralContext<'input> {}

impl<'input> CustomRuleContext<'input> for LiteralContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_literal
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_literal }
}
antlr4rust::tid! {LiteralContextExt<'a>}

impl<'input> LiteralContextExt<'input> {
    fn new(
        parent: Option<Rc<dyn CELParserContext<'input> + 'input>>,
        invoking_state: i32,
    ) -> Rc<LiteralContextAll<'input>> {
        Rc::new(LiteralContextAll::Error(
            BaseParserRuleContext::new_parser_ctx(
                parent,
                invoking_state,
                LiteralContextExt { ph: PhantomData },
            ),
        ))
    }
}

pub trait LiteralContextAttrs<'input>:
    CELParserContext<'input> + BorrowMut<LiteralContextExt<'input>>
{
}

impl<'input> LiteralContextAttrs<'input> for LiteralContext<'input> {}

pub type BytesContext<'input> = BaseParserRuleContext<'input, BytesContextExt<'input>>;

pub trait BytesContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token BYTES
    /// Returns `None` if there is no child corresponding to token BYTES
    fn BYTES(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_BYTES, 0)
    }
}

impl<'input> BytesContextAttrs<'input> for BytesContext<'input> {}

pub struct BytesContextExt<'input> {
    base: LiteralContextExt<'input>,
    pub tok: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {BytesContextExt<'a>}

impl<'input> CELParserContext<'input> for BytesContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for BytesContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Bytes(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Bytes(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for BytesContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Bytes(self);
    }
}

impl<'input> CustomRuleContext<'input> for BytesContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_literal
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_literal }
}

impl<'input> Borrow<LiteralContextExt<'input>> for BytesContext<'input> {
    fn borrow(&self) -> &LiteralContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LiteralContextExt<'input>> for BytesContext<'input> {
    fn borrow_mut(&mut self) -> &mut LiteralContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LiteralContextAttrs<'input> for BytesContext<'input> {}

impl<'input> BytesContextExt<'input> {
    fn new(ctx: &dyn LiteralContextAttrs<'input>) -> Rc<LiteralContextAll<'input>> {
        Rc::new(LiteralContextAll::BytesContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BytesContextExt {
                    tok: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type UintContext<'input> = BaseParserRuleContext<'input, UintContextExt<'input>>;

pub trait UintContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token NUM_UINT
    /// Returns `None` if there is no child corresponding to token NUM_UINT
    fn NUM_UINT(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_NUM_UINT, 0)
    }
}

impl<'input> UintContextAttrs<'input> for UintContext<'input> {}

pub struct UintContextExt<'input> {
    base: LiteralContextExt<'input>,
    pub tok: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {UintContextExt<'a>}

impl<'input> CELParserContext<'input> for UintContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for UintContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Uint(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Uint(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for UintContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Uint(self);
    }
}

impl<'input> CustomRuleContext<'input> for UintContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_literal
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_literal }
}

impl<'input> Borrow<LiteralContextExt<'input>> for UintContext<'input> {
    fn borrow(&self) -> &LiteralContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LiteralContextExt<'input>> for UintContext<'input> {
    fn borrow_mut(&mut self) -> &mut LiteralContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LiteralContextAttrs<'input> for UintContext<'input> {}

impl<'input> UintContextExt<'input> {
    fn new(ctx: &dyn LiteralContextAttrs<'input>) -> Rc<LiteralContextAll<'input>> {
        Rc::new(LiteralContextAll::UintContext(
            BaseParserRuleContext::copy_from(
                ctx,
                UintContextExt {
                    tok: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type NullContext<'input> = BaseParserRuleContext<'input, NullContextExt<'input>>;

pub trait NullContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token NUL
    /// Returns `None` if there is no child corresponding to token NUL
    fn NUL(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_NUL, 0)
    }
}

impl<'input> NullContextAttrs<'input> for NullContext<'input> {}

pub struct NullContextExt<'input> {
    base: LiteralContextExt<'input>,
    pub tok: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {NullContextExt<'a>}

impl<'input> CELParserContext<'input> for NullContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for NullContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Null(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Null(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for NullContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Null(self);
    }
}

impl<'input> CustomRuleContext<'input> for NullContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_literal
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_literal }
}

impl<'input> Borrow<LiteralContextExt<'input>> for NullContext<'input> {
    fn borrow(&self) -> &LiteralContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LiteralContextExt<'input>> for NullContext<'input> {
    fn borrow_mut(&mut self) -> &mut LiteralContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LiteralContextAttrs<'input> for NullContext<'input> {}

impl<'input> NullContextExt<'input> {
    fn new(ctx: &dyn LiteralContextAttrs<'input>) -> Rc<LiteralContextAll<'input>> {
        Rc::new(LiteralContextAll::NullContext(
            BaseParserRuleContext::copy_from(
                ctx,
                NullContextExt {
                    tok: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type BoolFalseContext<'input> = BaseParserRuleContext<'input, BoolFalseContextExt<'input>>;

pub trait BoolFalseContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token CEL_FALSE
    /// Returns `None` if there is no child corresponding to token CEL_FALSE
    fn CEL_FALSE(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_CEL_FALSE, 0)
    }
}

impl<'input> BoolFalseContextAttrs<'input> for BoolFalseContext<'input> {}

pub struct BoolFalseContextExt<'input> {
    base: LiteralContextExt<'input>,
    pub tok: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {BoolFalseContextExt<'a>}

impl<'input> CELParserContext<'input> for BoolFalseContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for BoolFalseContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_BoolFalse(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_BoolFalse(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for BoolFalseContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_BoolFalse(self);
    }
}

impl<'input> CustomRuleContext<'input> for BoolFalseContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_literal
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_literal }
}

impl<'input> Borrow<LiteralContextExt<'input>> for BoolFalseContext<'input> {
    fn borrow(&self) -> &LiteralContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LiteralContextExt<'input>> for BoolFalseContext<'input> {
    fn borrow_mut(&mut self) -> &mut LiteralContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LiteralContextAttrs<'input> for BoolFalseContext<'input> {}

impl<'input> BoolFalseContextExt<'input> {
    fn new(ctx: &dyn LiteralContextAttrs<'input>) -> Rc<LiteralContextAll<'input>> {
        Rc::new(LiteralContextAll::BoolFalseContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BoolFalseContextExt {
                    tok: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type StringContext<'input> = BaseParserRuleContext<'input, StringContextExt<'input>>;

pub trait StringContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token STRING
    /// Returns `None` if there is no child corresponding to token STRING
    fn STRING(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_STRING, 0)
    }
}

impl<'input> StringContextAttrs<'input> for StringContext<'input> {}

pub struct StringContextExt<'input> {
    base: LiteralContextExt<'input>,
    pub tok: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {StringContextExt<'a>}

impl<'input> CELParserContext<'input> for StringContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for StringContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_String(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_String(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for StringContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_String(self);
    }
}

impl<'input> CustomRuleContext<'input> for StringContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_literal
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_literal }
}

impl<'input> Borrow<LiteralContextExt<'input>> for StringContext<'input> {
    fn borrow(&self) -> &LiteralContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LiteralContextExt<'input>> for StringContext<'input> {
    fn borrow_mut(&mut self) -> &mut LiteralContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LiteralContextAttrs<'input> for StringContext<'input> {}

impl<'input> StringContextExt<'input> {
    fn new(ctx: &dyn LiteralContextAttrs<'input>) -> Rc<LiteralContextAll<'input>> {
        Rc::new(LiteralContextAll::StringContext(
            BaseParserRuleContext::copy_from(
                ctx,
                StringContextExt {
                    tok: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type DoubleContext<'input> = BaseParserRuleContext<'input, DoubleContextExt<'input>>;

pub trait DoubleContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token NUM_FLOAT
    /// Returns `None` if there is no child corresponding to token NUM_FLOAT
    fn NUM_FLOAT(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_NUM_FLOAT, 0)
    }
    /// Retrieves first TerminalNode corresponding to token MINUS
    /// Returns `None` if there is no child corresponding to token MINUS
    fn MINUS(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_MINUS, 0)
    }
}

impl<'input> DoubleContextAttrs<'input> for DoubleContext<'input> {}

pub struct DoubleContextExt<'input> {
    base: LiteralContextExt<'input>,
    pub sign: Option<TokenType<'input>>,
    pub tok: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {DoubleContextExt<'a>}

impl<'input> CELParserContext<'input> for DoubleContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for DoubleContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Double(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Double(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for DoubleContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Double(self);
    }
}

impl<'input> CustomRuleContext<'input> for DoubleContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_literal
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_literal }
}

impl<'input> Borrow<LiteralContextExt<'input>> for DoubleContext<'input> {
    fn borrow(&self) -> &LiteralContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LiteralContextExt<'input>> for DoubleContext<'input> {
    fn borrow_mut(&mut self) -> &mut LiteralContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LiteralContextAttrs<'input> for DoubleContext<'input> {}

impl<'input> DoubleContextExt<'input> {
    fn new(ctx: &dyn LiteralContextAttrs<'input>) -> Rc<LiteralContextAll<'input>> {
        Rc::new(LiteralContextAll::DoubleContext(
            BaseParserRuleContext::copy_from(
                ctx,
                DoubleContextExt {
                    sign: None,
                    tok: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type BoolTrueContext<'input> = BaseParserRuleContext<'input, BoolTrueContextExt<'input>>;

pub trait BoolTrueContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token CEL_TRUE
    /// Returns `None` if there is no child corresponding to token CEL_TRUE
    fn CEL_TRUE(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_CEL_TRUE, 0)
    }
}

impl<'input> BoolTrueContextAttrs<'input> for BoolTrueContext<'input> {}

pub struct BoolTrueContextExt<'input> {
    base: LiteralContextExt<'input>,
    pub tok: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {BoolTrueContextExt<'a>}

impl<'input> CELParserContext<'input> for BoolTrueContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for BoolTrueContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_BoolTrue(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_BoolTrue(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for BoolTrueContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_BoolTrue(self);
    }
}

impl<'input> CustomRuleContext<'input> for BoolTrueContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_literal
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_literal }
}

impl<'input> Borrow<LiteralContextExt<'input>> for BoolTrueContext<'input> {
    fn borrow(&self) -> &LiteralContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LiteralContextExt<'input>> for BoolTrueContext<'input> {
    fn borrow_mut(&mut self) -> &mut LiteralContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LiteralContextAttrs<'input> for BoolTrueContext<'input> {}

impl<'input> BoolTrueContextExt<'input> {
    fn new(ctx: &dyn LiteralContextAttrs<'input>) -> Rc<LiteralContextAll<'input>> {
        Rc::new(LiteralContextAll::BoolTrueContext(
            BaseParserRuleContext::copy_from(
                ctx,
                BoolTrueContextExt {
                    tok: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

pub type IntContext<'input> = BaseParserRuleContext<'input, IntContextExt<'input>>;

pub trait IntContextAttrs<'input>: CELParserContext<'input> {
    /// Retrieves first TerminalNode corresponding to token NUM_INT
    /// Returns `None` if there is no child corresponding to token NUM_INT
    fn NUM_INT(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_NUM_INT, 0)
    }
    /// Retrieves first TerminalNode corresponding to token MINUS
    /// Returns `None` if there is no child corresponding to token MINUS
    fn MINUS(&self) -> Option<Rc<TerminalNode<'input, CELParserContextType>>>
    where
        Self: Sized,
    {
        self.get_token(CEL_MINUS, 0)
    }
}

impl<'input> IntContextAttrs<'input> for IntContext<'input> {}

pub struct IntContextExt<'input> {
    base: LiteralContextExt<'input>,
    pub sign: Option<TokenType<'input>>,
    pub tok: Option<TokenType<'input>>,
    ph: PhantomData<&'input str>,
}

antlr4rust::tid! {IntContextExt<'a>}

impl<'input> CELParserContext<'input> for IntContext<'input> {}

impl<'input, 'a> Listenable<dyn CELListener<'input> + 'a> for IntContext<'input> {
    fn enter(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.enter_every_rule(self)?;
        listener.enter_Int(self);
        Ok(())
    }
    fn exit(&self, listener: &mut (dyn CELListener<'input> + 'a)) -> Result<(), ANTLRError> {
        listener.exit_Int(self);
        listener.exit_every_rule(self)?;
        Ok(())
    }
}

impl<'input, 'a> Visitable<dyn CELVisitor<'input> + 'a> for IntContext<'input> {
    fn accept(&self, visitor: &mut (dyn CELVisitor<'input> + 'a)) {
        visitor.visit_Int(self);
    }
}

impl<'input> CustomRuleContext<'input> for IntContextExt<'input> {
    type TF = LocalTokenFactory<'input>;
    type Ctx = CELParserContextType;
    fn get_rule_index(&self) -> usize {
        RULE_literal
    }
    // fn type_rule_index() -> usize where Self: Sized { RULE_literal }
}

impl<'input> Borrow<LiteralContextExt<'input>> for IntContext<'input> {
    fn borrow(&self) -> &LiteralContextExt<'input> {
        &self.base
    }
}
impl<'input> BorrowMut<LiteralContextExt<'input>> for IntContext<'input> {
    fn borrow_mut(&mut self) -> &mut LiteralContextExt<'input> {
        &mut self.base
    }
}

impl<'input> LiteralContextAttrs<'input> for IntContext<'input> {}

impl<'input> IntContextExt<'input> {
    fn new(ctx: &dyn LiteralContextAttrs<'input>) -> Rc<LiteralContextAll<'input>> {
        Rc::new(LiteralContextAll::IntContext(
            BaseParserRuleContext::copy_from(
                ctx,
                IntContextExt {
                    sign: None,
                    tok: None,
                    base: ctx.borrow().clone(),
                    ph: PhantomData,
                },
            ),
        ))
    }
}

impl<'input, I> CELParser<'input, I>
where
    I: TokenStream<'input, TF = LocalTokenFactory<'input>> + TidAble<'input>,
{
    pub fn literal(&mut self) -> Result<Rc<LiteralContextAll<'input>>, ANTLRError> {
        let mut recog = self;
        let _parentctx = recog.ctx.take();
        let mut _localctx = LiteralContextExt::new(_parentctx.clone(), recog.base.get_state());
        recog.base.enter_rule(_localctx.clone(), 32, RULE_literal);
        let mut _localctx: Rc<LiteralContextAll> = _localctx;
        let mut _la: i32 = -1;
        let result: Result<(), ANTLRError> = (|| {
            recog.base.set_state(256);
            recog.err_handler.sync(&mut recog.base)?;
            match recog.interpreter.adaptive_predict(35, &mut recog.base)? {
                1 => {
                    let tmp = IntContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 1)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(243);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == CEL_MINUS {
                            {
                                recog.base.set_state(242);
                                let tmp =
                                    recog.base.match_token(CEL_MINUS, &mut recog.err_handler)?;
                                if let LiteralContextAll::IntContext(ctx) =
                                    cast_mut::<_, LiteralContextAll>(&mut _localctx)
                                {
                                    ctx.sign = Some(tmp.clone());
                                } else {
                                    unreachable!("cant cast");
                                }
                            }
                        }

                        recog.base.set_state(245);
                        let tmp = recog
                            .base
                            .match_token(CEL_NUM_INT, &mut recog.err_handler)?;
                        if let LiteralContextAll::IntContext(ctx) =
                            cast_mut::<_, LiteralContextAll>(&mut _localctx)
                        {
                            ctx.tok = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                2 => {
                    let tmp = UintContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 2)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(246);
                        let tmp = recog
                            .base
                            .match_token(CEL_NUM_UINT, &mut recog.err_handler)?;
                        if let LiteralContextAll::UintContext(ctx) =
                            cast_mut::<_, LiteralContextAll>(&mut _localctx)
                        {
                            ctx.tok = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                3 => {
                    let tmp = DoubleContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 3)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(248);
                        recog.err_handler.sync(&mut recog.base)?;
                        _la = recog.base.input.la(1);
                        if _la == CEL_MINUS {
                            {
                                recog.base.set_state(247);
                                let tmp =
                                    recog.base.match_token(CEL_MINUS, &mut recog.err_handler)?;
                                if let LiteralContextAll::DoubleContext(ctx) =
                                    cast_mut::<_, LiteralContextAll>(&mut _localctx)
                                {
                                    ctx.sign = Some(tmp.clone());
                                } else {
                                    unreachable!("cant cast");
                                }
                            }
                        }

                        recog.base.set_state(250);
                        let tmp = recog
                            .base
                            .match_token(CEL_NUM_FLOAT, &mut recog.err_handler)?;
                        if let LiteralContextAll::DoubleContext(ctx) =
                            cast_mut::<_, LiteralContextAll>(&mut _localctx)
                        {
                            ctx.tok = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                4 => {
                    let tmp = StringContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 4)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(251);
                        let tmp = recog.base.match_token(CEL_STRING, &mut recog.err_handler)?;
                        if let LiteralContextAll::StringContext(ctx) =
                            cast_mut::<_, LiteralContextAll>(&mut _localctx)
                        {
                            ctx.tok = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                5 => {
                    let tmp = BytesContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 5)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(252);
                        let tmp = recog.base.match_token(CEL_BYTES, &mut recog.err_handler)?;
                        if let LiteralContextAll::BytesContext(ctx) =
                            cast_mut::<_, LiteralContextAll>(&mut _localctx)
                        {
                            ctx.tok = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                6 => {
                    let tmp = BoolTrueContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 6)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(253);
                        let tmp = recog
                            .base
                            .match_token(CEL_CEL_TRUE, &mut recog.err_handler)?;
                        if let LiteralContextAll::BoolTrueContext(ctx) =
                            cast_mut::<_, LiteralContextAll>(&mut _localctx)
                        {
                            ctx.tok = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                7 => {
                    let tmp = BoolFalseContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 7)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(254);
                        let tmp = recog
                            .base
                            .match_token(CEL_CEL_FALSE, &mut recog.err_handler)?;
                        if let LiteralContextAll::BoolFalseContext(ctx) =
                            cast_mut::<_, LiteralContextAll>(&mut _localctx)
                        {
                            ctx.tok = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }
                8 => {
                    let tmp = NullContextExt::new(&**_localctx);
                    recog.base.enter_outer_alt(Some(tmp.clone()), 8)?;
                    _localctx = tmp;
                    {
                        recog.base.set_state(255);
                        let tmp = recog.base.match_token(CEL_NUL, &mut recog.err_handler)?;
                        if let LiteralContextAll::NullContext(ctx) =
                            cast_mut::<_, LiteralContextAll>(&mut _localctx)
                        {
                            ctx.tok = Some(tmp.clone());
                        } else {
                            unreachable!("cant cast");
                        }
                    }
                }

                _ => {}
            }
            Ok(())
        })();
        match result {
            Ok(_) => {}
            Err(e @ ANTLRError::FallThrough(_)) => return Err(e),
            Err(ref re) => {
                //_localctx.exception = re;
                recog.err_handler.report_error(&mut recog.base, re);
                recog.err_handler.recover(&mut recog.base, re)?;
            }
        }
        recog.base.exit_rule()?;

        Ok(_localctx)
    }
}
lazy_static! {
    static ref _ATN: Arc<ATN> =
        Arc::new(ATNDeserializer::new(None).deserialize(&mut _serializedATN.iter()));
    static ref _decision_to_DFA: Arc<Vec<antlr4rust::RwLock<DFA>>> = {
        let mut dfa = Vec::new();
        let size = _ATN.decision_to_state.len() as i32;
        for i in 0..size {
            dfa.push(DFA::new(_ATN.clone(), _ATN.get_decision_state(i), i).into())
        }
        Arc::new(dfa)
    };
    static ref _serializedATN: Vec<i32> = vec![
        4, 1, 37, 259, 2, 0, 7, 0, 2, 1, 7, 1, 2, 2, 7, 2, 2, 3, 7, 3, 2, 4, 7, 4, 2, 5, 7, 5, 2,
        6, 7, 6, 2, 7, 7, 7, 2, 8, 7, 8, 2, 9, 7, 9, 2, 10, 7, 10, 2, 11, 7, 11, 2, 12, 7, 12, 2,
        13, 7, 13, 2, 14, 7, 14, 2, 15, 7, 15, 2, 16, 7, 16, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 1, 3, 1, 44, 8, 1, 1, 2, 1, 2, 1, 2, 5, 2, 49, 8, 2, 10, 2, 12, 2, 52, 9, 2, 1,
        3, 1, 3, 1, 3, 5, 3, 57, 8, 3, 10, 3, 12, 3, 60, 9, 3, 1, 4, 1, 4, 1, 4, 1, 4, 1, 4, 1, 4,
        5, 4, 68, 8, 4, 10, 4, 12, 4, 71, 9, 4, 1, 5, 1, 5, 1, 5, 1, 5, 1, 5, 1, 5, 1, 5, 1, 5, 1,
        5, 5, 5, 82, 8, 5, 10, 5, 12, 5, 85, 9, 5, 1, 6, 1, 6, 4, 6, 89, 8, 6, 11, 6, 12, 6, 90, 1,
        6, 1, 6, 4, 6, 95, 8, 6, 11, 6, 12, 6, 96, 1, 6, 3, 6, 100, 8, 6, 1, 7, 1, 7, 1, 7, 1, 7,
        1, 7, 1, 7, 3, 7, 108, 8, 7, 1, 7, 1, 7, 1, 7, 1, 7, 1, 7, 1, 7, 3, 7, 116, 8, 7, 1, 7, 1,
        7, 1, 7, 1, 7, 3, 7, 122, 8, 7, 1, 7, 1, 7, 1, 7, 5, 7, 127, 8, 7, 10, 7, 12, 7, 130, 9, 7,
        1, 8, 3, 8, 133, 8, 8, 1, 8, 1, 8, 3, 8, 137, 8, 8, 1, 8, 1, 8, 1, 8, 3, 8, 142, 8, 8, 1,
        8, 1, 8, 1, 8, 1, 8, 1, 8, 1, 8, 1, 8, 3, 8, 151, 8, 8, 1, 8, 3, 8, 154, 8, 8, 1, 8, 1, 8,
        1, 8, 3, 8, 159, 8, 8, 1, 8, 3, 8, 162, 8, 8, 1, 8, 1, 8, 3, 8, 166, 8, 8, 1, 8, 1, 8, 1,
        8, 5, 8, 171, 8, 8, 10, 8, 12, 8, 174, 9, 8, 1, 8, 1, 8, 3, 8, 178, 8, 8, 1, 8, 3, 8, 181,
        8, 8, 1, 8, 1, 8, 3, 8, 185, 8, 8, 1, 9, 1, 9, 1, 9, 5, 9, 190, 8, 9, 10, 9, 12, 9, 193, 9,
        9, 1, 10, 1, 10, 1, 10, 5, 10, 198, 8, 10, 10, 10, 12, 10, 201, 9, 10, 1, 11, 1, 11, 1, 11,
        1, 11, 1, 11, 1, 11, 1, 11, 1, 11, 5, 11, 211, 8, 11, 10, 11, 12, 11, 214, 9, 11, 1, 12, 3,
        12, 217, 8, 12, 1, 12, 1, 12, 1, 13, 1, 13, 1, 13, 1, 13, 1, 13, 1, 13, 1, 13, 1, 13, 5,
        13, 229, 8, 13, 10, 13, 12, 13, 232, 9, 13, 1, 14, 1, 14, 3, 14, 236, 8, 14, 1, 15, 3, 15,
        239, 8, 15, 1, 15, 1, 15, 1, 16, 3, 16, 244, 8, 16, 1, 16, 1, 16, 1, 16, 3, 16, 249, 8, 16,
        1, 16, 1, 16, 1, 16, 1, 16, 1, 16, 1, 16, 3, 16, 257, 8, 16, 1, 16, 0, 3, 8, 10, 14, 17, 0,
        2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 32, 0, 3, 1, 0, 1, 7, 1, 0, 23, 25,
        2, 0, 18, 18, 22, 22, 290, 0, 34, 1, 0, 0, 0, 2, 37, 1, 0, 0, 0, 4, 45, 1, 0, 0, 0, 6, 53,
        1, 0, 0, 0, 8, 61, 1, 0, 0, 0, 10, 72, 1, 0, 0, 0, 12, 99, 1, 0, 0, 0, 14, 101, 1, 0, 0, 0,
        16, 184, 1, 0, 0, 0, 18, 186, 1, 0, 0, 0, 20, 194, 1, 0, 0, 0, 22, 202, 1, 0, 0, 0, 24,
        216, 1, 0, 0, 0, 26, 220, 1, 0, 0, 0, 28, 235, 1, 0, 0, 0, 30, 238, 1, 0, 0, 0, 32, 256, 1,
        0, 0, 0, 34, 35, 3, 2, 1, 0, 35, 36, 5, 0, 0, 1, 36, 1, 1, 0, 0, 0, 37, 43, 3, 4, 2, 0, 38,
        39, 5, 20, 0, 0, 39, 40, 3, 4, 2, 0, 40, 41, 5, 21, 0, 0, 41, 42, 3, 2, 1, 0, 42, 44, 1, 0,
        0, 0, 43, 38, 1, 0, 0, 0, 43, 44, 1, 0, 0, 0, 44, 3, 1, 0, 0, 0, 45, 50, 3, 6, 3, 0, 46,
        47, 5, 9, 0, 0, 47, 49, 3, 6, 3, 0, 48, 46, 1, 0, 0, 0, 49, 52, 1, 0, 0, 0, 50, 48, 1, 0,
        0, 0, 50, 51, 1, 0, 0, 0, 51, 5, 1, 0, 0, 0, 52, 50, 1, 0, 0, 0, 53, 58, 3, 8, 4, 0, 54,
        55, 5, 8, 0, 0, 55, 57, 3, 8, 4, 0, 56, 54, 1, 0, 0, 0, 57, 60, 1, 0, 0, 0, 58, 56, 1, 0,
        0, 0, 58, 59, 1, 0, 0, 0, 59, 7, 1, 0, 0, 0, 60, 58, 1, 0, 0, 0, 61, 62, 6, 4, -1, 0, 62,
        63, 3, 10, 5, 0, 63, 69, 1, 0, 0, 0, 64, 65, 10, 1, 0, 0, 65, 66, 7, 0, 0, 0, 66, 68, 3, 8,
        4, 2, 67, 64, 1, 0, 0, 0, 68, 71, 1, 0, 0, 0, 69, 67, 1, 0, 0, 0, 69, 70, 1, 0, 0, 0, 70,
        9, 1, 0, 0, 0, 71, 69, 1, 0, 0, 0, 72, 73, 6, 5, -1, 0, 73, 74, 3, 12, 6, 0, 74, 83, 1, 0,
        0, 0, 75, 76, 10, 2, 0, 0, 76, 77, 7, 1, 0, 0, 77, 82, 3, 10, 5, 3, 78, 79, 10, 1, 0, 0,
        79, 80, 7, 2, 0, 0, 80, 82, 3, 10, 5, 2, 81, 75, 1, 0, 0, 0, 81, 78, 1, 0, 0, 0, 82, 85, 1,
        0, 0, 0, 83, 81, 1, 0, 0, 0, 83, 84, 1, 0, 0, 0, 84, 11, 1, 0, 0, 0, 85, 83, 1, 0, 0, 0,
        86, 100, 3, 14, 7, 0, 87, 89, 5, 19, 0, 0, 88, 87, 1, 0, 0, 0, 89, 90, 1, 0, 0, 0, 90, 88,
        1, 0, 0, 0, 90, 91, 1, 0, 0, 0, 91, 92, 1, 0, 0, 0, 92, 100, 3, 14, 7, 0, 93, 95, 5, 18, 0,
        0, 94, 93, 1, 0, 0, 0, 95, 96, 1, 0, 0, 0, 96, 94, 1, 0, 0, 0, 96, 97, 1, 0, 0, 0, 97, 98,
        1, 0, 0, 0, 98, 100, 3, 14, 7, 0, 99, 86, 1, 0, 0, 0, 99, 88, 1, 0, 0, 0, 99, 94, 1, 0, 0,
        0, 100, 13, 1, 0, 0, 0, 101, 102, 6, 7, -1, 0, 102, 103, 3, 16, 8, 0, 103, 128, 1, 0, 0, 0,
        104, 105, 10, 3, 0, 0, 105, 107, 5, 16, 0, 0, 106, 108, 5, 20, 0, 0, 107, 106, 1, 0, 0, 0,
        107, 108, 1, 0, 0, 0, 108, 109, 1, 0, 0, 0, 109, 127, 3, 28, 14, 0, 110, 111, 10, 2, 0, 0,
        111, 112, 5, 16, 0, 0, 112, 113, 5, 36, 0, 0, 113, 115, 5, 14, 0, 0, 114, 116, 3, 18, 9, 0,
        115, 114, 1, 0, 0, 0, 115, 116, 1, 0, 0, 0, 116, 117, 1, 0, 0, 0, 117, 127, 5, 15, 0, 0,
        118, 119, 10, 1, 0, 0, 119, 121, 5, 10, 0, 0, 120, 122, 5, 20, 0, 0, 121, 120, 1, 0, 0, 0,
        121, 122, 1, 0, 0, 0, 122, 123, 1, 0, 0, 0, 123, 124, 3, 2, 1, 0, 124, 125, 5, 11, 0, 0,
        125, 127, 1, 0, 0, 0, 126, 104, 1, 0, 0, 0, 126, 110, 1, 0, 0, 0, 126, 118, 1, 0, 0, 0,
        127, 130, 1, 0, 0, 0, 128, 126, 1, 0, 0, 0, 128, 129, 1, 0, 0, 0, 129, 15, 1, 0, 0, 0, 130,
        128, 1, 0, 0, 0, 131, 133, 5, 16, 0, 0, 132, 131, 1, 0, 0, 0, 132, 133, 1, 0, 0, 0, 133,
        134, 1, 0, 0, 0, 134, 185, 5, 36, 0, 0, 135, 137, 5, 16, 0, 0, 136, 135, 1, 0, 0, 0, 136,
        137, 1, 0, 0, 0, 137, 138, 1, 0, 0, 0, 138, 139, 5, 36, 0, 0, 139, 141, 5, 14, 0, 0, 140,
        142, 3, 18, 9, 0, 141, 140, 1, 0, 0, 0, 141, 142, 1, 0, 0, 0, 142, 143, 1, 0, 0, 0, 143,
        185, 5, 15, 0, 0, 144, 145, 5, 14, 0, 0, 145, 146, 3, 2, 1, 0, 146, 147, 5, 15, 0, 0, 147,
        185, 1, 0, 0, 0, 148, 150, 5, 10, 0, 0, 149, 151, 3, 20, 10, 0, 150, 149, 1, 0, 0, 0, 150,
        151, 1, 0, 0, 0, 151, 153, 1, 0, 0, 0, 152, 154, 5, 17, 0, 0, 153, 152, 1, 0, 0, 0, 153,
        154, 1, 0, 0, 0, 154, 155, 1, 0, 0, 0, 155, 185, 5, 11, 0, 0, 156, 158, 5, 12, 0, 0, 157,
        159, 3, 26, 13, 0, 158, 157, 1, 0, 0, 0, 158, 159, 1, 0, 0, 0, 159, 161, 1, 0, 0, 0, 160,
        162, 5, 17, 0, 0, 161, 160, 1, 0, 0, 0, 161, 162, 1, 0, 0, 0, 162, 163, 1, 0, 0, 0, 163,
        185, 5, 13, 0, 0, 164, 166, 5, 16, 0, 0, 165, 164, 1, 0, 0, 0, 165, 166, 1, 0, 0, 0, 166,
        167, 1, 0, 0, 0, 167, 172, 5, 36, 0, 0, 168, 169, 5, 16, 0, 0, 169, 171, 5, 36, 0, 0, 170,
        168, 1, 0, 0, 0, 171, 174, 1, 0, 0, 0, 172, 170, 1, 0, 0, 0, 172, 173, 1, 0, 0, 0, 173,
        175, 1, 0, 0, 0, 174, 172, 1, 0, 0, 0, 175, 177, 5, 12, 0, 0, 176, 178, 3, 22, 11, 0, 177,
        176, 1, 0, 0, 0, 177, 178, 1, 0, 0, 0, 178, 180, 1, 0, 0, 0, 179, 181, 5, 17, 0, 0, 180,
        179, 1, 0, 0, 0, 180, 181, 1, 0, 0, 0, 181, 182, 1, 0, 0, 0, 182, 185, 5, 13, 0, 0, 183,
        185, 3, 32, 16, 0, 184, 132, 1, 0, 0, 0, 184, 136, 1, 0, 0, 0, 184, 144, 1, 0, 0, 0, 184,
        148, 1, 0, 0, 0, 184, 156, 1, 0, 0, 0, 184, 165, 1, 0, 0, 0, 184, 183, 1, 0, 0, 0, 185, 17,
        1, 0, 0, 0, 186, 191, 3, 2, 1, 0, 187, 188, 5, 17, 0, 0, 188, 190, 3, 2, 1, 0, 189, 187, 1,
        0, 0, 0, 190, 193, 1, 0, 0, 0, 191, 189, 1, 0, 0, 0, 191, 192, 1, 0, 0, 0, 192, 19, 1, 0,
        0, 0, 193, 191, 1, 0, 0, 0, 194, 199, 3, 30, 15, 0, 195, 196, 5, 17, 0, 0, 196, 198, 3, 30,
        15, 0, 197, 195, 1, 0, 0, 0, 198, 201, 1, 0, 0, 0, 199, 197, 1, 0, 0, 0, 199, 200, 1, 0, 0,
        0, 200, 21, 1, 0, 0, 0, 201, 199, 1, 0, 0, 0, 202, 203, 3, 24, 12, 0, 203, 204, 5, 21, 0,
        0, 204, 212, 3, 2, 1, 0, 205, 206, 5, 17, 0, 0, 206, 207, 3, 24, 12, 0, 207, 208, 5, 21, 0,
        0, 208, 209, 3, 2, 1, 0, 209, 211, 1, 0, 0, 0, 210, 205, 1, 0, 0, 0, 211, 214, 1, 0, 0, 0,
        212, 210, 1, 0, 0, 0, 212, 213, 1, 0, 0, 0, 213, 23, 1, 0, 0, 0, 214, 212, 1, 0, 0, 0, 215,
        217, 5, 20, 0, 0, 216, 215, 1, 0, 0, 0, 216, 217, 1, 0, 0, 0, 217, 218, 1, 0, 0, 0, 218,
        219, 3, 28, 14, 0, 219, 25, 1, 0, 0, 0, 220, 221, 3, 30, 15, 0, 221, 222, 5, 21, 0, 0, 222,
        230, 3, 2, 1, 0, 223, 224, 5, 17, 0, 0, 224, 225, 3, 30, 15, 0, 225, 226, 5, 21, 0, 0, 226,
        227, 3, 2, 1, 0, 227, 229, 1, 0, 0, 0, 228, 223, 1, 0, 0, 0, 229, 232, 1, 0, 0, 0, 230,
        228, 1, 0, 0, 0, 230, 231, 1, 0, 0, 0, 231, 27, 1, 0, 0, 0, 232, 230, 1, 0, 0, 0, 233, 236,
        5, 36, 0, 0, 234, 236, 5, 37, 0, 0, 235, 233, 1, 0, 0, 0, 235, 234, 1, 0, 0, 0, 236, 29, 1,
        0, 0, 0, 237, 239, 5, 20, 0, 0, 238, 237, 1, 0, 0, 0, 238, 239, 1, 0, 0, 0, 239, 240, 1, 0,
        0, 0, 240, 241, 3, 2, 1, 0, 241, 31, 1, 0, 0, 0, 242, 244, 5, 18, 0, 0, 243, 242, 1, 0, 0,
        0, 243, 244, 1, 0, 0, 0, 244, 245, 1, 0, 0, 0, 245, 257, 5, 32, 0, 0, 246, 257, 5, 33, 0,
        0, 247, 249, 5, 18, 0, 0, 248, 247, 1, 0, 0, 0, 248, 249, 1, 0, 0, 0, 249, 250, 1, 0, 0, 0,
        250, 257, 5, 31, 0, 0, 251, 257, 5, 34, 0, 0, 252, 257, 5, 35, 0, 0, 253, 257, 5, 26, 0, 0,
        254, 257, 5, 27, 0, 0, 255, 257, 5, 28, 0, 0, 256, 243, 1, 0, 0, 0, 256, 246, 1, 0, 0, 0,
        256, 248, 1, 0, 0, 0, 256, 251, 1, 0, 0, 0, 256, 252, 1, 0, 0, 0, 256, 253, 1, 0, 0, 0,
        256, 254, 1, 0, 0, 0, 256, 255, 1, 0, 0, 0, 257, 33, 1, 0, 0, 0, 36, 43, 50, 58, 69, 81,
        83, 90, 96, 99, 107, 115, 121, 126, 128, 132, 136, 141, 150, 153, 158, 161, 165, 172, 177,
        180, 184, 191, 199, 212, 216, 230, 235, 238, 243, 248, 256
    ];
}
