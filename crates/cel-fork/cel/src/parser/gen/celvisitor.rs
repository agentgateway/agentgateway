#![allow(nonstandard_style)]
// Generated from CEL.g4 by ANTLR 4.13.2
use antlr4rust::tree::{ParseTreeVisitor, ParseTreeVisitorCompat};

use super::celparser::*;

/// This interface defines a complete generic visitor for a parse tree produced
/// by {@link CELParser}.
pub trait CELVisitor<'input>: ParseTreeVisitor<'input, CELParserContextType> {
    /// Visit a parse tree produced by {@link CELParser#start}.
    /// @param ctx the parse tree
    fn visit_start(&mut self, ctx: &StartContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#expr}.
    /// @param ctx the parse tree
    fn visit_expr(&mut self, ctx: &ExprContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#conditionalOr}.
    /// @param ctx the parse tree
    fn visit_conditionalOr(&mut self, ctx: &ConditionalOrContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#conditionalAnd}.
    /// @param ctx the parse tree
    fn visit_conditionalAnd(&mut self, ctx: &ConditionalAndContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#relation}.
    /// @param ctx the parse tree
    fn visit_relation(&mut self, ctx: &RelationContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#calc}.
    /// @param ctx the parse tree
    fn visit_calc(&mut self, ctx: &CalcContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code MemberExpr}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn visit_MemberExpr(&mut self, ctx: &MemberExprContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code LogicalNot}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn visit_LogicalNot(&mut self, ctx: &LogicalNotContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Negate}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn visit_Negate(&mut self, ctx: &NegateContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code MemberCall}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn visit_MemberCall(&mut self, ctx: &MemberCallContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Select}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn visit_Select(&mut self, ctx: &SelectContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code PrimaryExpr}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn visit_PrimaryExpr(&mut self, ctx: &PrimaryExprContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Index}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn visit_Index(&mut self, ctx: &IndexContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Ident}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_Ident(&mut self, ctx: &IdentContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code GlobalCall}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_GlobalCall(&mut self, ctx: &GlobalCallContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Nested}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_Nested(&mut self, ctx: &NestedContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code CreateList}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_CreateList(&mut self, ctx: &CreateListContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code CreateStruct}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_CreateStruct(&mut self, ctx: &CreateStructContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code CreateMessage}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_CreateMessage(&mut self, ctx: &CreateMessageContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code ConstantLiteral}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_ConstantLiteral(&mut self, ctx: &ConstantLiteralContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#exprList}.
    /// @param ctx the parse tree
    fn visit_exprList(&mut self, ctx: &ExprListContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#listInit}.
    /// @param ctx the parse tree
    fn visit_listInit(&mut self, ctx: &ListInitContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#field_initializer_list}.
    /// @param ctx the parse tree
    fn visit_field_initializer_list(&mut self, ctx: &Field_initializer_listContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#optField}.
    /// @param ctx the parse tree
    fn visit_optField(&mut self, ctx: &OptFieldContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#mapInitializerList}.
    /// @param ctx the parse tree
    fn visit_mapInitializerList(&mut self, ctx: &MapInitializerListContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code SimpleIdentifier}
    /// labeled alternative in {@link CELParser#escapeIdent}.
    /// @param ctx the parse tree
    fn visit_SimpleIdentifier(&mut self, ctx: &SimpleIdentifierContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code EscapedIdentifier}
    /// labeled alternative in {@link CELParser#escapeIdent}.
    /// @param ctx the parse tree
    fn visit_EscapedIdentifier(&mut self, ctx: &EscapedIdentifierContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#optExpr}.
    /// @param ctx the parse tree
    fn visit_optExpr(&mut self, ctx: &OptExprContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Int}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Int(&mut self, ctx: &IntContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Uint}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Uint(&mut self, ctx: &UintContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Double}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Double(&mut self, ctx: &DoubleContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code String}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_String(&mut self, ctx: &StringContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Bytes}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Bytes(&mut self, ctx: &BytesContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code BoolTrue}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_BoolTrue(&mut self, ctx: &BoolTrueContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code BoolFalse}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_BoolFalse(&mut self, ctx: &BoolFalseContext<'input>) {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Null}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Null(&mut self, ctx: &NullContext<'input>) {
        self.visit_children(ctx)
    }
}

pub trait CELVisitorCompat<'input>:
    ParseTreeVisitorCompat<'input, Node = CELParserContextType>
{
    /// Visit a parse tree produced by {@link CELParser#start}.
    /// @param ctx the parse tree
    fn visit_start(&mut self, ctx: &StartContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#expr}.
    /// @param ctx the parse tree
    fn visit_expr(&mut self, ctx: &ExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#conditionalOr}.
    /// @param ctx the parse tree
    fn visit_conditionalOr(&mut self, ctx: &ConditionalOrContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#conditionalAnd}.
    /// @param ctx the parse tree
    fn visit_conditionalAnd(&mut self, ctx: &ConditionalAndContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#relation}.
    /// @param ctx the parse tree
    fn visit_relation(&mut self, ctx: &RelationContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#calc}.
    /// @param ctx the parse tree
    fn visit_calc(&mut self, ctx: &CalcContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code MemberExpr}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn visit_MemberExpr(&mut self, ctx: &MemberExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code LogicalNot}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn visit_LogicalNot(&mut self, ctx: &LogicalNotContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Negate}
    /// labeled alternative in {@link CELParser#unary}.
    /// @param ctx the parse tree
    fn visit_Negate(&mut self, ctx: &NegateContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code MemberCall}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn visit_MemberCall(&mut self, ctx: &MemberCallContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Select}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn visit_Select(&mut self, ctx: &SelectContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code PrimaryExpr}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn visit_PrimaryExpr(&mut self, ctx: &PrimaryExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Index}
    /// labeled alternative in {@link CELParser#member}.
    /// @param ctx the parse tree
    fn visit_Index(&mut self, ctx: &IndexContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Ident}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_Ident(&mut self, ctx: &IdentContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code GlobalCall}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_GlobalCall(&mut self, ctx: &GlobalCallContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Nested}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_Nested(&mut self, ctx: &NestedContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code CreateList}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_CreateList(&mut self, ctx: &CreateListContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code CreateStruct}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_CreateStruct(&mut self, ctx: &CreateStructContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code CreateMessage}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_CreateMessage(&mut self, ctx: &CreateMessageContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code ConstantLiteral}
    /// labeled alternative in {@link CELParser#primary}.
    /// @param ctx the parse tree
    fn visit_ConstantLiteral(&mut self, ctx: &ConstantLiteralContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#exprList}.
    /// @param ctx the parse tree
    fn visit_exprList(&mut self, ctx: &ExprListContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#listInit}.
    /// @param ctx the parse tree
    fn visit_listInit(&mut self, ctx: &ListInitContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#field_initializer_list}.
    /// @param ctx the parse tree
    fn visit_field_initializer_list(
        &mut self,
        ctx: &Field_initializer_listContext<'input>,
    ) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#optField}.
    /// @param ctx the parse tree
    fn visit_optField(&mut self, ctx: &OptFieldContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#mapInitializerList}.
    /// @param ctx the parse tree
    fn visit_mapInitializerList(
        &mut self,
        ctx: &MapInitializerListContext<'input>,
    ) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code SimpleIdentifier}
    /// labeled alternative in {@link CELParser#escapeIdent}.
    /// @param ctx the parse tree
    fn visit_SimpleIdentifier(&mut self, ctx: &SimpleIdentifierContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code EscapedIdentifier}
    /// labeled alternative in {@link CELParser#escapeIdent}.
    /// @param ctx the parse tree
    fn visit_EscapedIdentifier(&mut self, ctx: &EscapedIdentifierContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by {@link CELParser#optExpr}.
    /// @param ctx the parse tree
    fn visit_optExpr(&mut self, ctx: &OptExprContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Int}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Int(&mut self, ctx: &IntContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Uint}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Uint(&mut self, ctx: &UintContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Double}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Double(&mut self, ctx: &DoubleContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code String}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_String(&mut self, ctx: &StringContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Bytes}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Bytes(&mut self, ctx: &BytesContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code BoolTrue}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_BoolTrue(&mut self, ctx: &BoolTrueContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code BoolFalse}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_BoolFalse(&mut self, ctx: &BoolFalseContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }

    /// Visit a parse tree produced by the {@code Null}
    /// labeled alternative in {@link CELParser#literal}.
    /// @param ctx the parse tree
    fn visit_Null(&mut self, ctx: &NullContext<'input>) -> Self::Return {
        self.visit_children(ctx)
    }
}

impl<'input, T> CELVisitor<'input> for T
where
    T: CELVisitorCompat<'input>,
{
    fn visit_start(&mut self, ctx: &StartContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_start(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_expr(&mut self, ctx: &ExprContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_expr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_conditionalOr(&mut self, ctx: &ConditionalOrContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_conditionalOr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_conditionalAnd(&mut self, ctx: &ConditionalAndContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_conditionalAnd(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_relation(&mut self, ctx: &RelationContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_relation(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_calc(&mut self, ctx: &CalcContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_calc(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_MemberExpr(&mut self, ctx: &MemberExprContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_MemberExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_LogicalNot(&mut self, ctx: &LogicalNotContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_LogicalNot(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Negate(&mut self, ctx: &NegateContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Negate(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_MemberCall(&mut self, ctx: &MemberCallContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_MemberCall(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Select(&mut self, ctx: &SelectContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Select(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_PrimaryExpr(&mut self, ctx: &PrimaryExprContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_PrimaryExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Index(&mut self, ctx: &IndexContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Index(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Ident(&mut self, ctx: &IdentContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Ident(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_GlobalCall(&mut self, ctx: &GlobalCallContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_GlobalCall(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Nested(&mut self, ctx: &NestedContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Nested(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_CreateList(&mut self, ctx: &CreateListContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_CreateList(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_CreateStruct(&mut self, ctx: &CreateStructContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_CreateStruct(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_CreateMessage(&mut self, ctx: &CreateMessageContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_CreateMessage(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_ConstantLiteral(&mut self, ctx: &ConstantLiteralContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_ConstantLiteral(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_exprList(&mut self, ctx: &ExprListContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_exprList(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_listInit(&mut self, ctx: &ListInitContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_listInit(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_field_initializer_list(&mut self, ctx: &Field_initializer_listContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_field_initializer_list(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_optField(&mut self, ctx: &OptFieldContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_optField(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_mapInitializerList(&mut self, ctx: &MapInitializerListContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_mapInitializerList(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_SimpleIdentifier(&mut self, ctx: &SimpleIdentifierContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_SimpleIdentifier(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_EscapedIdentifier(&mut self, ctx: &EscapedIdentifierContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_EscapedIdentifier(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_optExpr(&mut self, ctx: &OptExprContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_optExpr(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Int(&mut self, ctx: &IntContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Int(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Uint(&mut self, ctx: &UintContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Uint(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Double(&mut self, ctx: &DoubleContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Double(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_String(&mut self, ctx: &StringContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_String(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Bytes(&mut self, ctx: &BytesContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Bytes(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BoolTrue(&mut self, ctx: &BoolTrueContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_BoolTrue(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_BoolFalse(&mut self, ctx: &BoolFalseContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_BoolFalse(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }

    fn visit_Null(&mut self, ctx: &NullContext<'input>) {
        let result = <Self as CELVisitorCompat>::visit_Null(self, ctx);
        *<Self as ParseTreeVisitorCompat>::temp_result(self) = result;
    }
}
