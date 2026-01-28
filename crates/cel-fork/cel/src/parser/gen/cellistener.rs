#![allow(nonstandard_style)]
// Generated from CEL.g4 by ANTLR 4.13.2
use antlr4rust::tree::ParseTreeListener;

use super::celparser::*;

pub trait CELListener<'input>: ParseTreeListener<'input, CELParserContextType> {
    /// Enter a parse tree produced by {@link CELParser#start}.
    /// @param ctx the parse tree
    fn enter_start(&mut self, _ctx: &StartContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#start}.
    /// @param ctx the parse tree
    fn exit_start(&mut self, _ctx: &StartContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#expr}.
    /// @param ctx the parse tree
    fn enter_expr(&mut self, _ctx: &ExprContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#expr}.
    /// @param ctx the parse tree
    fn exit_expr(&mut self, _ctx: &ExprContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#conditionalOr}.
    /// @param ctx the parse tree
    fn enter_conditionalOr(&mut self, _ctx: &ConditionalOrContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#conditionalOr}.
    /// @param ctx the parse tree
    fn exit_conditionalOr(&mut self, _ctx: &ConditionalOrContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#conditionalAnd}.
    /// @param ctx the parse tree
    fn enter_conditionalAnd(&mut self, _ctx: &ConditionalAndContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#conditionalAnd}.
    /// @param ctx the parse tree
    fn exit_conditionalAnd(&mut self, _ctx: &ConditionalAndContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#relation}.
    /// @param ctx the parse tree
    fn enter_relation(&mut self, _ctx: &RelationContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#relation}.
    /// @param ctx the parse tree
    fn exit_relation(&mut self, _ctx: &RelationContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#calc}.
    /// @param ctx the parse tree
    fn enter_calc(&mut self, _ctx: &CalcContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#calc}.
    /// @param ctx the parse tree
    fn exit_calc(&mut self, _ctx: &CalcContext<'input>) {}
    /// Enter a parse tree produced by the {@code MemberExpr}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn enter_MemberExpr(&mut self, _ctx: &MemberExprContext<'input>) {}
    /// Exit a parse tree produced by the {@code MemberExpr}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn exit_MemberExpr(&mut self, _ctx: &MemberExprContext<'input>) {}
    /// Enter a parse tree produced by the {@code LogicalNot}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn enter_LogicalNot(&mut self, _ctx: &LogicalNotContext<'input>) {}
    /// Exit a parse tree produced by the {@code LogicalNot}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn exit_LogicalNot(&mut self, _ctx: &LogicalNotContext<'input>) {}
    /// Enter a parse tree produced by the {@code Negate}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn enter_Negate(&mut self, _ctx: &NegateContext<'input>) {}
    /// Exit a parse tree produced by the {@code Negate}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn exit_Negate(&mut self, _ctx: &NegateContext<'input>) {}
    /// Enter a parse tree produced by the {@code MemberCall}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn enter_MemberCall(&mut self, _ctx: &MemberCallContext<'input>) {}
    /// Exit a parse tree produced by the {@code MemberCall}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn exit_MemberCall(&mut self, _ctx: &MemberCallContext<'input>) {}
    /// Enter a parse tree produced by the {@code Select}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn enter_Select(&mut self, _ctx: &SelectContext<'input>) {}
    /// Exit a parse tree produced by the {@code Select}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn exit_Select(&mut self, _ctx: &SelectContext<'input>) {}
    /// Enter a parse tree produced by the {@code PrimaryExpr}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn enter_PrimaryExpr(&mut self, _ctx: &PrimaryExprContext<'input>) {}
    /// Exit a parse tree produced by the {@code PrimaryExpr}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn exit_PrimaryExpr(&mut self, _ctx: &PrimaryExprContext<'input>) {}
    /// Enter a parse tree produced by the {@code Index}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn enter_Index(&mut self, _ctx: &IndexContext<'input>) {}
    /// Exit a parse tree produced by the {@code Index}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn exit_Index(&mut self, _ctx: &IndexContext<'input>) {}
    /// Enter a parse tree produced by the {@code Ident}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn enter_Ident(&mut self, _ctx: &IdentContext<'input>) {}
    /// Exit a parse tree produced by the {@code Ident}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn exit_Ident(&mut self, _ctx: &IdentContext<'input>) {}
    /// Enter a parse tree produced by the {@code GlobalCall}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn enter_GlobalCall(&mut self, _ctx: &GlobalCallContext<'input>) {}
    /// Exit a parse tree produced by the {@code GlobalCall}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn exit_GlobalCall(&mut self, _ctx: &GlobalCallContext<'input>) {}
    /// Enter a parse tree produced by the {@code Nested}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn enter_Nested(&mut self, _ctx: &NestedContext<'input>) {}
    /// Exit a parse tree produced by the {@code Nested}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn exit_Nested(&mut self, _ctx: &NestedContext<'input>) {}
    /// Enter a parse tree produced by the {@code CreateList}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn enter_CreateList(&mut self, _ctx: &CreateListContext<'input>) {}
    /// Exit a parse tree produced by the {@code CreateList}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn exit_CreateList(&mut self, _ctx: &CreateListContext<'input>) {}
    /// Enter a parse tree produced by the {@code CreateStruct}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn enter_CreateStruct(&mut self, _ctx: &CreateStructContext<'input>) {}
    /// Exit a parse tree produced by the {@code CreateStruct}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn exit_CreateStruct(&mut self, _ctx: &CreateStructContext<'input>) {}
    /// Enter a parse tree produced by the {@code CreateMessage}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn enter_CreateMessage(&mut self, _ctx: &CreateMessageContext<'input>) {}
    /// Exit a parse tree produced by the {@code CreateMessage}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn exit_CreateMessage(&mut self, _ctx: &CreateMessageContext<'input>) {}
    /// Enter a parse tree produced by the {@code ConstantLiteral}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn enter_ConstantLiteral(&mut self, _ctx: &ConstantLiteralContext<'input>) {}
    /// Exit a parse tree produced by the {@code ConstantLiteral}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn exit_ConstantLiteral(&mut self, _ctx: &ConstantLiteralContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#exprList}.
    /// @param ctx the parse tree
    fn enter_exprList(&mut self, _ctx: &ExprListContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#exprList}.
    /// @param ctx the parse tree
    fn exit_exprList(&mut self, _ctx: &ExprListContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#listInit}.
    /// @param ctx the parse tree
    fn enter_listInit(&mut self, _ctx: &ListInitContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#listInit}.
    /// @param ctx the parse tree
    fn exit_listInit(&mut self, _ctx: &ListInitContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#field_initializer_list}.
    /// @param ctx the parse tree
    fn enter_field_initializer_list(&mut self, _ctx: &Field_initializer_listContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#field_initializer_list}.
    /// @param ctx the parse tree
    fn exit_field_initializer_list(&mut self, _ctx: &Field_initializer_listContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#optField}.
    /// @param ctx the parse tree
    fn enter_optField(&mut self, _ctx: &OptFieldContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#optField}.
    /// @param ctx the parse tree
    fn exit_optField(&mut self, _ctx: &OptFieldContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#mapInitializerList}.
    /// @param ctx the parse tree
    fn enter_mapInitializerList(&mut self, _ctx: &MapInitializerListContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#mapInitializerList}.
    /// @param ctx the parse tree
    fn exit_mapInitializerList(&mut self, _ctx: &MapInitializerListContext<'input>) {}
    /// Enter a parse tree produced by the {@code SimpleIdentifier}
    /// labeled alternative in {@link CELParser#escapeIdent}.
    /// @param ctx the parse tree
    fn enter_SimpleIdentifier(&mut self, _ctx: &SimpleIdentifierContext<'input>) {}
    /// Exit a parse tree produced by the {@code SimpleIdentifier}
    /// labeled alternative in {@link CELParser#escapeIdent}.
    /// @param ctx the parse tree
    fn exit_SimpleIdentifier(&mut self, _ctx: &SimpleIdentifierContext<'input>) {}
    /// Enter a parse tree produced by the {@code EscapedIdentifier}
    /// labeled alternative in {@link CELParser#escapeIdent}.
    /// @param ctx the parse tree
    fn enter_EscapedIdentifier(&mut self, _ctx: &EscapedIdentifierContext<'input>) {}
    /// Exit a parse tree produced by the {@code EscapedIdentifier}
    /// labeled alternative in {@link CELParser#escapeIdent}.
    /// @param ctx the parse tree
    fn exit_EscapedIdentifier(&mut self, _ctx: &EscapedIdentifierContext<'input>) {}
    /// Enter a parse tree produced by {@link CELParser#optExpr}.
    /// @param ctx the parse tree
    fn enter_optExpr(&mut self, _ctx: &OptExprContext<'input>) {}
    /// Exit a parse tree produced by {@link CELParser#optExpr}.
    /// @param ctx the parse tree
    fn exit_optExpr(&mut self, _ctx: &OptExprContext<'input>) {}
    /// Enter a parse tree produced by the {@code Int}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn enter_Int(&mut self, _ctx: &IntContext<'input>) {}
    /// Exit a parse tree produced by the {@code Int}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn exit_Int(&mut self, _ctx: &IntContext<'input>) {}
    /// Enter a parse tree produced by the {@code Uint}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn enter_Uint(&mut self, _ctx: &UintContext<'input>) {}
    /// Exit a parse tree produced by the {@code Uint}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn exit_Uint(&mut self, _ctx: &UintContext<'input>) {}
    /// Enter a parse tree produced by the {@code Double}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn enter_Double(&mut self, _ctx: &DoubleContext<'input>) {}
    /// Exit a parse tree produced by the {@code Double}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn exit_Double(&mut self, _ctx: &DoubleContext<'input>) {}
    /// Enter a parse tree produced by the {@code String}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn enter_String(&mut self, _ctx: &StringContext<'input>) {}
    /// Exit a parse tree produced by the {@code String}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn exit_String(&mut self, _ctx: &StringContext<'input>) {}
    /// Enter a parse tree produced by the {@code Bytes}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn enter_Bytes(&mut self, _ctx: &BytesContext<'input>) {}
    /// Exit a parse tree produced by the {@code Bytes}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn exit_Bytes(&mut self, _ctx: &BytesContext<'input>) {}
    /// Enter a parse tree produced by the {@code BoolTrue}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn enter_BoolTrue(&mut self, _ctx: &BoolTrueContext<'input>) {}
    /// Exit a parse tree produced by the {@code BoolTrue}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn exit_BoolTrue(&mut self, _ctx: &BoolTrueContext<'input>) {}
    /// Enter a parse tree produced by the {@code BoolFalse}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn enter_BoolFalse(&mut self, _ctx: &BoolFalseContext<'input>) {}
    /// Exit a parse tree produced by the {@code BoolFalse}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn exit_BoolFalse(&mut self, _ctx: &BoolFalseContext<'input>) {}
    /// Enter a parse tree produced by the {@code Null}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn enter_Null(&mut self, _ctx: &NullContext<'input>) {}
    /// Exit a parse tree produced by the {@code Null}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn exit_Null(&mut self, _ctx: &NullContext<'input>) {}
}

antlr4rust::coerce_from! { 'input : CELListener<'input> }
