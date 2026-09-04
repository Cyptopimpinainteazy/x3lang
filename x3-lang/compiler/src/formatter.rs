//! Pretty-printer for X3 AST.
//!
//! Walks the parsed AST and produces formatted, human-readable source code.

use std::fmt::Write;
use x3_lang_ast::ast::*;
use x3_lang_common::Spanned;

pub struct X3Formatter {
    output: String,
    indent_level: usize,
}

impl X3Formatter {
    pub fn new() -> Self {
        X3Formatter {
            output: String::new(),
            indent_level: 0,
        }
    }

    pub fn format_program(&mut self, program: &Program) -> String {
        self.output.clear();
        self.indent_level = 0;
        for item in &program.items {
            self.format_item(&item.node);
            self.output.push('\n');
        }
        self.output.clone()
    }

    fn indent(&mut self) {
        self.indent_level += 1;
    }

    fn dedent(&mut self) {
        self.indent_level = self.indent_level.saturating_sub(1);
    }

    fn write_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.output.push_str("    ");
        }
    }

    fn write(&mut self, s: &str) {
        self.output.push_str(s);
    }

    fn write_line(&mut self, s: &str) {
        self.write_indent();
        self.output.push_str(s);
        self.output.push('\n');
    }

    fn format_item(&mut self, item: &Item) {
        match item {
            Item::Use(decl) => self.format_use(decl),
            Item::Import(decl) => self.format_import(decl),
            Item::Const(decl) => self.format_const(decl),
            Item::Function(f) => self.format_function(f),
            Item::Agent(a) => self.format_agent(a),
            Item::Struct(s) => self.format_struct(s),
            Item::Enum(e) => self.format_enum(e),
            Item::Bridge(b) => self.format_bridge(b),
            Item::AtomicSwap(a) => self.format_atomic_swap(a),
            Item::Strategy(s) => self.format_strategy(s),
            Item::Proposal(p) => self.format_proposal(p),
            Item::IntentDecl(i) => self.format_intent(i),
            Item::GpuBlock(g) => self.format_gpu(g),
            Item::SimulateDecl(s) => self.format_simulate(s),
            Item::ScheduledTask(t) => self.format_scheduled(t),
            Item::SubscriptionDecl(s) => self.format_subscription(s),
            Item::Mod(m) => self.format_mod(m),
            Item::VmDecl(v) => self.format_vm(v),
            Item::SolverMarket(m) => self.format_solver_market(m),
            Item::RelayerSwarm(r) => self.format_relayer_swarm(r),
            Item::RpcQuorum(q) => self.format_rpc_quorum(q),
            Item::RiskPolicy(p) => self.format_risk_policy(p),
            Item::PrivacyBlock(p) => self.format_privacy_block(p),
            Item::InvariantDecl(i) => self.format_invariant(i),
            Item::ErrorDecl(e) => self.format_error(e),
            Item::FinalityPolicy(f) => self.format_finality_policy(f),
            Item::ProofsRequired(p) => self.format_proofs_required(p),
            Item::VmTarget(t) => self.format_vm_target(t),
        }
    }

    fn format_use(&mut self, decl: &UseDecl) {
        self.write("use ");
        for (i, seg) in decl.path.iter().enumerate() {
            if i > 0 {
                self.write("::");
            }
            self.write(seg.as_str());
        }
        if let Some(alias) = &decl.alias {
            self.write(" as ");
            self.write(alias.as_str());
        }
        self.write(";\n");
    }

    fn format_import(&mut self, decl: &ImportDecl) {
        self.write("import ");
        for (i, seg) in decl.module.iter().enumerate() {
            if i > 0 {
                self.write("::");
            }
            self.write(seg.as_str());
        }
        if let Some(alias) = &decl.as_alias {
            self.write(" as ");
            self.write(alias.as_str());
        }
        self.write(";\n");
    }

    fn format_const(&mut self, decl: &ConstDecl) {
        self.write("const ");
        self.write(decl.name.as_str());
        if let Some(ty) = &decl.ty {
            self.write(": ");
            self.format_type(ty);
        }
        self.write(" = ");
        self.format_expression(&decl.value);
        self.write(";\n");
    }

    fn format_function(&mut self, f: &Function) {
        if f.is_async {
            self.write("async ");
        }
        self.write("fn ");
        self.write(f.name.as_str());
        self.write("(");
        for (i, p) in f.params.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            if p.is_mut {
                self.write("mut ");
            }
            if let Some(name) = &p.name {
                self.write(name.as_str());
            }
            if let Some(ty) = &p.ty {
                self.write(": ");
                self.format_type(ty);
            }
        }
        self.write(")");
        if let Some(ret) = &f.ret {
            self.write(" -> ");
            self.format_type(ret);
        }
        self.write(" ");
        self.format_block(&f.body, true);
    }

    fn format_agent(&mut self, a: &Agent) {
        self.write("agent ");
        self.write(a.name.as_str());
        self.write(" {\n");
        self.indent();
        for m in &a.methods {
            self.format_function(&m.node);
        }
        for s in &a.strategies {
            self.write_indent();
            self.write("strategy ");
            self.write(s.node.name.as_str());
            self.write("(");
            for (i, p) in s.node.params.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                if let Some(name) = &p.name {
                    self.write(name.as_str());
                }
            }
            self.write(") ");
            self.format_block(&s.node.body, true);
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_struct(&mut self, s: &StructDecl) {
        self.write("struct ");
        self.write(s.name.as_str());
        self.write(" {\n");
        self.indent();
        for f in &s.fields {
            self.write_indent();
            self.write(f.name.as_str());
            self.write(": ");
            self.format_type(&f.ty);
            self.write(",\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_enum(&mut self, e: &EnumDecl) {
        self.write("enum ");
        self.write(e.name.as_str());
        self.write(" {\n");
        self.indent();
        for v in &e.variants {
            self.write_indent();
            self.write(v.name.as_str());
            if let Some(ty) = &v.payload {
                self.write("(");
                self.format_type(ty);
                self.write(")");
            }
            self.write(",\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_mod(&mut self, m: &ModDecl) {
        self.write("mod ");
        self.write(m.name.as_str());
        self.write(" {\n");
        self.indent();
        for item in &m.items {
            self.format_item(&item.node);
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_bridge(&mut self, b: &BridgeDecl) {
        self.write("bridge ");
        self.write(b.name.as_str());
        self.write(" ");
        self.format_asset_ref(&b.from_asset);
        self.write(" to ");
        self.format_asset_ref(&b.to_asset);
        self.write(" {\n");
        self.indent();
        for s in &b.body {
            self.format_statement(s);
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_atomic_swap(&mut self, a: &AtomicSwapDecl) {
        self.write("atomic swap ");
        self.format_asset_ref(&a.from_asset);
        self.write(" -> ");
        self.format_asset_ref(&a.to_asset);
        self.write(" {\n");
        self.indent();
        for stmt in &a.body {
            self.format_statement(stmt);
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_strategy(&mut self, s: &CrossChainStrategy) {
        self.write("strategy ");
        self.write(s.name.as_str());
        self.write(" {\n");
        self.indent();
        for stmt in &s.body {
            self.format_statement(stmt);
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_proposal(&mut self, p: &ProposalDecl) {
        self.write("proposal ");
        self.write(p.name.as_str());
        if let Some(title) = &p.title {
            self.write(": ");
            self.format_expression(title);
        }
        self.write(" {\n");
        self.indent();
        for stmt in &p.body {
            self.format_statement(stmt);
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_intent(&mut self, i: &IntentDecl) {
        self.write("intent ");
        self.write(i.name.as_str());
        self.write(" {\n");
        self.indent();
        if !i.constraints.is_empty() {
            self.write_indent();
            self.write("[");
            for (idx, c) in i.constraints.iter().enumerate() {
                if idx > 0 {
                    self.write(", ");
                }
                self.format_expression(c);
            }
            self.write("]\n");
        }
        for stmt in &i.body.stmts {
            self.format_statement(stmt);
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_gpu(&mut self, g: &GpuBlock) {
        self.write("gpu");
        if g.is_simd {
            self.write(" simd");
        }
        self.write(" ");
        self.format_block(&g.body, true);
    }

    fn format_simulate(&mut self, s: &SimulateDecl) {
        self.write("simulate ");
        self.write(s.name.as_str());
        self.write(" ");
        self.format_block(&s.body, true);
        if let Some(r) = &s.receipt {
            self.write(" receipt: ");
            self.write(r.as_str());
        }
        self.write("\n");
    }

    fn format_scheduled(&mut self, t: &ScheduledTask) {
        self.write("scheduled ");
        self.write(t.name.as_str());
        self.write(": ");
        self.write(&t.period_blocks.to_string());
        self.write(" ");
        self.format_block(&t.body, true);
    }

    fn format_subscription(&mut self, s: &SubscriptionDecl) {
        self.write("subscription ");
        self.write(s.name.as_str());
        self.write(": ");
        self.write(&s.amount.to_string());
        self.write(" ");
        self.format_block(&s.body, true);
    }

    fn format_vm(&mut self, v: &VmDecl) {
        self.write("vm ");
        self.write(v.chain.as_str());
        self.write(" { adapter: ");
        self.write(v.adapter.as_str());
        if let Some(f) = &v.finality {
            self.write(", finality: ");
            self.write(f.as_str());
        }
        self.write(" };\n");
    }

    fn format_solver_market(&mut self, m: &SolverMarket) {
        self.write("solver_market ");
        self.write(m.mode.as_str());
        self.write(" { min_reputation: ");
        self.write(&m.min_reputation.to_string());
        self.write(" };\n");
    }

    fn format_relayer_swarm(&mut self, r: &RelayerSwarm) {
        self.write("relayers { quorum: ");
        self.write(&r.quorum_numerator.to_string());
        self.write("_of_");
        self.write(&r.quorum_denominator.to_string());
        self.write(", relayers: [");
        for (i, rel) in r.relayers.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            self.write(rel.as_str());
        }
        self.write("] };\n");
    }

    fn format_rpc_quorum(&mut self, q: &RpcQuorum) {
        self.write("rpc_quorum ");
        self.write(q.source.as_str());
        self.write(" { require: ");
        self.write(&q.require_numerator.to_string());
        self.write("_of_");
        self.write(&q.require_denominator.to_string());
        self.write(" };\n");
    }

    fn format_risk_policy(&mut self, p: &RiskPolicy) {
        self.write("risk_policy { max_slippage: ");
        self.write(&p.max_slippage.to_string());
        if let Some(pos) = &p.max_position {
            self.write(", max_position: ");
            self.write(&pos.to_string());
        }
        self.write(" };\n");
    }

    fn format_privacy_block(&mut self, p: &PrivacyBlock) {
        self.write("privacy { hide_route_until_commit: ");
        self.write(if p.hide_route_until_commit { "true" } else { "false" });
        self.write(", reveal_on: ");
        self.write(p.reveal_on.as_str());
        self.write(", encrypted: ");
        self.write(if p.encrypted { "true" } else { "false" });
        self.write(" };\n");
    }

    fn format_invariant(&mut self, i: &InvariantDecl) {
        self.write("invariant ");
        self.write(i.name.as_str());
        self.write(": ");
        self.write(i.assert_expr.as_str());
        self.write(";\n");
    }

    fn format_error(&mut self, e: &ErrorDecl) {
        self.write("error ");
        self.write(e.name.as_str());
        self.write(";\n");
    }

    fn format_finality_policy(&mut self, f: &FinalityPolicy) {
        self.write("finality_policy ");
        self.write(f.mode.as_str());
        self.write(" { chain: ");
        self.write(f.chain.as_str());
        self.write(", require: ");
        self.write(f.requirement.as_str());
        self.write(" };\n");
    }

    fn format_proofs_required(&mut self, p: &ProofsRequired) {
        self.write("proofs required { ");
        for (i, proof) in p.proofs.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            self.write(proof.as_str());
        }
        self.write(" };\n");
    }

    fn format_vm_target(&mut self, t: &VmTarget) {
        self.write("target ");
        self.write(t.vm.as_str());
        self.write(" { adapter: ");
        self.write(t.adapter.as_str());
        if let Some(c) = &t.contract {
            self.write(", contract: ");
            self.write(c.as_str());
        }
        self.write(" };\n");
    }

    fn format_block(&mut self, block: &Block, _braces_same_line: bool) {
        self.write("{\n");
        self.indent();
        for stmt in &block.stmts {
            self.format_statement(stmt);
        }
        self.dedent();
        self.write_indent();
        self.write("}\n");
    }

    fn format_statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Let { name, ty, expr, is_mut } => {
                self.write_indent();
                self.write("let ");
                if *is_mut {
                    self.write("mut ");
                }
                self.write(name.as_str());
                if let Some(t) = ty {
                    self.write(": ");
                    self.format_type(t);
                }
                if let Some(e) = expr {
                    self.write(" = ");
                    self.format_expression(e);
                }
                self.write(";\n");
            }
            Statement::Expr(expr) => {
                self.write_indent();
                self.format_expression(expr);
                self.write(";\n");
            }
            Statement::Return(expr) => {
                self.write_indent();
                self.write("return");
                if let Some(e) = expr {
                    self.write(" ");
                    self.format_expression(e);
                }
                self.write(";\n");
            }
            Statement::If {
                cond,
                then_block,
                else_block,
            } => {
                self.write_indent();
                self.write("if ");
                self.format_expression(cond);
                self.write(" ");
                self.format_block(then_block, true);
                if let Some(else_b) = else_block {
                    self.write_indent();
                    self.write("else ");
                    self.format_block(else_b, true);
                }
            }
            Statement::While { cond, body } => {
                self.write_indent();
                self.write("while ");
                self.format_expression(cond);
                self.write(" ");
                self.format_block(body, true);
            }
            Statement::For {
                pattern,
                iterable,
                body,
            } => {
                self.write_indent();
                self.write("for ");
                self.format_pattern(pattern);
                self.write(" in ");
                self.format_expression(iterable);
                self.write(" ");
                self.format_block(body, true);
            }
            Statement::Loop(body) => {
                self.write_indent();
                self.write("loop ");
                self.format_block(body, true);
            }
            Statement::Atomic(atomic) => {
                self.write_indent();
                self.write("atomic ");
                self.format_block(&atomic.body, true);
            }
            Statement::Emit(emit) => {
                self.write_indent();
                self.write("emit ");
                self.write(emit.name.as_str());
                self.write("(");
                for (i, p) in emit.payload.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.format_expression(p);
                }
                self.write(");\n");
            }
            Statement::Lock {
                chain,
                asset,
                amount,
                from,
            } => {
                self.write_indent();
                self.write("lock ");
                self.write(chain.as_str());
                self.write(".");
                self.write(asset.name.as_str());
                self.write(" amount ");
                self.format_expression(amount);
                self.write(" from ");
                self.format_expression(from);
                self.write(";\n");
            }
            Statement::Mint { asset, amount, to } => {
                self.write_indent();
                self.write("mint ");
                self.format_asset_ref(asset);
                self.write(" amount ");
                self.format_expression(amount);
                self.write(" to ");
                self.format_expression(to);
                self.write(";\n");
            }
            Statement::Burn { asset, amount, from } => {
                self.write_indent();
                self.write("burn ");
                self.format_asset_ref(asset);
                self.write(" amount ");
                self.format_expression(amount);
                self.write(" from ");
                self.format_expression(from);
                self.write(";\n");
            }
            Statement::Release { chain, asset, to } => {
                self.write_indent();
                self.write("release ");
                self.write(chain.as_str());
                self.write(".");
                self.write(asset.name.as_str());
                self.write(" to ");
                self.format_expression(to);
                self.write(";\n");
            }
            Statement::Swap {
                from,
                to,
                route,
                min_output,
                dex,
            } => {
                self.write_indent();
                if let Some(Expression::Literal(LiteralExpr::String(s))) = dex {
                    self.write(s.as_str());
                    self.write(" ");
                }
                self.write("swap ");
                self.format_asset_ref(from);
                self.write(" -> ");
                self.format_asset_ref(to);
                if let Some(amt) = route {
                    self.write(" amount ");
                    self.format_expression(amt);
                }
                if let Some(min) = min_output {
                    self.write(" min_output ");
                    self.format_expression(min);
                }
                self.write(";\n");
            }
            Statement::Bridge {
                via,
                from,
                to,
                amount,
                receiver,
                ..
            } => {
                self.write_indent();
                self.write("bridge ");
                self.write(via.as_str());
                self.write(" ");
                self.format_asset_ref(from);
                self.write(" -> ");
                self.format_asset_ref(to);
                self.write(" amount ");
                self.format_expression(amount);
                self.write(" receiver ");
                self.format_expression(receiver);
                self.write(";\n");
            }
            Statement::Require(guard) => {
                self.write_indent();
                self.write("require ");
                self.format_require_kind(&guard.kind);
                if let Some(subject) = &guard.subject {
                    self.write(" ");
                    self.write(subject.as_str());
                }
                self.write(" ");
                self.format_expression(&guard.value);
                self.write(";\n");
            }
            Statement::OnFail(action) => {
                self.write_indent();
                self.write("on_fail ");
                match action {
                    FailureAction::Rollback => self.write("rollback;\n"),
                    FailureAction::Refund(expr) => {
                        self.write("refund ");
                        self.format_expression(expr);
                        self.write(";\n");
                    }
                    FailureAction::Halt => self.write("halt;\n"),
                    FailureAction::Quarantine => self.write("quarantine;\n"),
                }
            }
            Statement::OnTimeout { duration, action } => {
                self.write_indent();
                self.write("on_timeout ");
                self.format_expression(duration);
                self.write(" ");
                match action {
                    FailureAction::Rollback => self.write("rollback;\n"),
                    FailureAction::Refund(expr) => {
                        self.write("refund ");
                        self.format_expression(expr);
                        self.write(";\n");
                    }
                    FailureAction::Halt => self.write("halt;\n"),
                    FailureAction::Quarantine => self.write("quarantine;\n"),
                }
            }
            Statement::Break => self.write_line("break;"),
            Statement::Continue => self.write_line("continue;"),
            Statement::Snapshot => self.write_line("snapshot();"),
            Statement::SelfDestruct => self.write_line("self_destruct();"),
            Statement::Pause => self.write_line("pause();"),
            Statement::Resume => self.write_line("resume();"),
            Statement::Diff { before, after } => {
                self.write_indent();
                self.write("diff(");
                self.format_expression(before);
                self.write(", ");
                self.format_expression(after);
                self.write(");\n");
            }
            Statement::CrdtOp(op) => {
                self.write_indent();
                let kind = match op.kind {
                    CrdtOpKind::Get => "crdt_get",
                    CrdtOpKind::Set => "crdt_set",
                    CrdtOpKind::Append => "crdt_append",
                    CrdtOpKind::Merge => "crdt_merge",
                };
                self.write(kind);
                self.write("(");
                self.format_expression(&op.key);
                if let Some(v) = &op.value {
                    self.write(", ");
                    self.format_expression(v);
                }
                self.write(");\n");
            }
            Statement::Migrate { new_contract } => {
                self.write_indent();
                self.write("migrate_and_destroy(");
                self.format_expression(new_contract);
                self.write(");\n");
            }
            Statement::ZkVerify {
                proof,
                public_input,
                key,
            } => {
                self.write_indent();
                self.write("verify_zk(");
                self.format_expression(proof);
                self.write(", ");
                self.format_expression(public_input);
                self.write(", ");
                self.format_expression(key);
                self.write(");\n");
            }
            Statement::Pathfind { from, to, max_depth } => {
                self.write_indent();
                self.write("pathfind(");
                self.format_expression(from);
                self.write(", ");
                self.format_expression(to);
                self.write(", ");
                self.format_expression(max_depth);
                self.write(");\n");
            }
            Statement::OracleRequest { token, reward } => {
                self.write_indent();
                self.write("oracle_request(");
                self.format_expression(token);
                self.write(", ");
                self.format_expression(reward);
                self.write(");\n");
            }
            _ => {
                self.write_indent();
                self.write_line(&format!("// <statement {:?}>", std::mem::discriminant(stmt)));
            }
        }
    }

    fn format_expression(&mut self, expr: &Expression) {
        match expr {
            Expression::Literal(lit) => self.format_literal(lit),
            Expression::Ident(name) => self.write(name.as_str()),
            Expression::Binary { op, lhs, rhs } => {
                self.format_expression(lhs);
                self.write(&format!(" {:?} ", op));
                self.format_expression(rhs);
            }
            Expression::Unary { op, expr: inner } => {
                self.write(match op {
                    x3_lang_common::UnOp::Neg => "-",
                    x3_lang_common::UnOp::Not => "!",
                    x3_lang_common::UnOp::Deref => "*",
                    x3_lang_common::UnOp::Ref => "&",
                    x3_lang_common::UnOp::RefMut => "&mut ",
                });
                self.format_expression(inner);
            }
            Expression::Call { callee, args } => {
                self.format_expression(callee);
                self.write("(");
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.format_expression(a);
                }
                self.write(")");
            }
            Expression::MethodCall { receiver, method, args } => {
                self.format_expression(receiver);
                self.write(".");
                self.write(method.as_str());
                self.write("(");
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.format_expression(a);
                }
                self.write(")");
            }
            Expression::FieldAccess { target, field } => {
                self.format_expression(target);
                self.write(".");
                self.write(field.as_str());
            }
            Expression::Index { target, index } => {
                self.format_expression(target);
                self.write("[");
                self.format_expression(index);
                self.write("]");
            }
            Expression::IfExpr {
                cond,
                then_block,
                else_block,
            } => {
                self.write("if ");
                self.format_expression(cond);
                self.write(" ");
                self.format_block(then_block, true);
                if let Some(eb) = else_block {
                    self.write(" else ");
                    self.format_block(eb, true);
                }
            }
            Expression::BlockExpr(block) => {
                self.format_block(block, true);
            }
            Expression::Closure { params, body, .. } => {
                self.write("|");
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    if let Some(name) = &p.name {
                        self.write(name.as_str());
                    }
                }
                self.write("| ");
                self.format_expression(body);
            }
            Expression::Await(inner) => {
                self.write("await ");
                self.format_expression(inner);
            }
            Expression::Async(inner) => {
                self.write("async ");
                self.format_expression(inner);
            }
            Expression::Match { expr: matchee, arms } => {
                self.write("match ");
                self.format_expression(matchee);
                self.write(" {\n");
                self.indent();
                for (pat, body) in arms {
                    self.write_indent();
                    self.format_pattern(pat);
                    self.write(" => ");
                    self.format_expression(body);
                    self.write(",\n");
                }
                self.dedent();
                self.write_indent();
                self.write("}");
            }
            Expression::Try(inner) => {
                self.write("try ");
                self.format_expression(inner);
            }
            Expression::Atomic(atomic) => {
                self.write("atomic ");
                self.format_block(&atomic.body, true);
            }
        }
    }

    fn format_literal(&mut self, lit: &LiteralExpr) {
        match lit {
            LiteralExpr::Int { value, base, suffix: _ } => match base {
                x3_lang_common::IntBase::Decimal => self.write(&value.to_string()),
                x3_lang_common::IntBase::Hex => self.write(&format!("0x{value:x}")),
                x3_lang_common::IntBase::Binary => self.write(&format!("0b{value:b}")),
                x3_lang_common::IntBase::Octal => self.write(&format!("0o{value:o}")),
            },
            LiteralExpr::Float { raw, .. } => self.write(raw.as_str()),
            LiteralExpr::String(s) => {
                self.write("\"");
                self.write(s.as_str());
                self.write("\"");
            }
            LiteralExpr::Bool(true) => self.write("true"),
            LiteralExpr::Bool(false) => self.write("false"),
            LiteralExpr::Unit => self.write("()"),
            LiteralExpr::Address(a) => {
                self.write("@");
                self.write(a.as_str());
            }
            LiteralExpr::Hash(h) => {
                self.write("#");
                self.write(h.as_str());
            }
            _ => self.write("/* literal */"),
        }
    }

    fn format_type(&mut self, ty: &TypeExpr) {
        match ty {
            TypeExpr::Path(path) => {
                for (i, seg) in path.iter().enumerate() {
                    if i > 0 {
                        self.write("::");
                    }
                    self.write(seg.as_str());
                }
            }
            TypeExpr::Array(inner, size) => {
                self.write("[");
                self.format_type(inner);
                if let Some(s) = size {
                    self.write("; ");
                    self.write(&s.to_string());
                }
                self.write("]");
            }
            TypeExpr::Tuple(types) => {
                self.write("(");
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.format_type(t);
                }
                self.write(")");
            }
            TypeExpr::Primitive(name) => self.write(name.as_str()),
            TypeExpr::Generic { base, args } => {
                self.format_type(base);
                self.write("<");
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.format_type(a);
                }
                self.write(">");
            }
            TypeExpr::Func { params, ret } => {
                self.write("fn(");
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.format_type(p);
                }
                self.write(") -> ");
                self.format_type(ret);
            }
            TypeExpr::Option(inner) => {
                self.write("Option<");
                self.format_type(inner);
                self.write(">");
            }
        }
    }

    fn format_pattern(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Wildcard => self.write("_"),
            Pattern::Ident(name) => self.write(name.as_str()),
            Pattern::Tuple(pats) => {
                self.write("(");
                for (i, p) in pats.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.format_pattern(p);
                }
                self.write(")");
            }
            Pattern::Literal(lit) => self.format_literal(lit),
        }
    }

    fn format_asset_ref(&mut self, asset: &AssetRef) {
        self.write(asset.chain.as_str());
        self.write(".");
        self.write(asset.name.as_str());
    }

    fn format_require_kind(&mut self, kind: &RequireKind) {
        match kind {
            RequireKind::Finality => self.write("finality"),
            RequireKind::Slippage => self.write("slippage"),
            RequireKind::Profit => self.write("profit"),
            RequireKind::InvariantCheck => self.write("invariant"),
            RequireKind::RiskScore => self.write("risk"),
            RequireKind::Nonce => self.write("nonce"),
            RequireKind::AuditGate => self.write("audit_gate"),
            RequireKind::BridgeLiquidity => self.write("bridge_liquidity"),
            RequireKind::CanonicalSupply => self.write("canonical_supply"),
            RequireKind::RelayerQuorum => self.write("relayer_quorum"),
            RequireKind::RouteScore => self.write("route_score"),
            RequireKind::SolverBond => self.write("solver_bond"),
            RequireKind::ProofComplete => self.write("proof_complete"),
            RequireKind::RefundPath => self.write("refund_path"),
            RequireKind::FinalityExplicit => self.write("finality_explicit"),
            RequireKind::VmSupported => self.write("vm_supported"),
            RequireKind::MainnetSafe => self.write("mainnet_safe"),
            RequireKind::Custom(sym) => self.write(sym.as_str()),
        }
    }
}

impl Default for X3Formatter {
    fn default() -> Self {
        Self::new()
    }
}
