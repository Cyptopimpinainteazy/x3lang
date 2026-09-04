use crate::ast::*;

pub enum WalkResult {
    Continue,
    Stop,
}

pub trait AstVisitor {
    fn enter_program(&mut self, _p: &Program) {}
    fn exit_program(&mut self, _p: &Program) {}
    fn enter_item(&mut self, _s: &spanned::Spanned<Item>) {}
    fn exit_item(&mut self, _s: &spanned::Spanned<Item>) {}
    fn visit_agent(&mut self, _a: &Agent) {}
    fn visit_function(&mut self, _f: &Function) {}
    fn visit_struct(&mut self, _s: &StructDecl) {}
    fn visit_enum(&mut self, _e: &EnumDecl) {}
    // Cross-chain visitors (default no-op)
    fn visit_bridge(&mut self, _b: &BridgeDecl) {}
    fn visit_atomic_swap(&mut self, _a: &AtomicSwapDecl) {}
    fn visit_cross_chain_strategy(&mut self, _s: &CrossChainStrategy) {}
    fn visit_proposal(&mut self, _p: &ProposalDecl) {}
}

// Helper module alias for Spanned to avoid cyclic dependencies in imports
mod spanned {
    pub use x3_lang_common::Spanned;
}
