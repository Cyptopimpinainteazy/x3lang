//! Pretty-printer for X3 AST.
//!
//! Walks the parsed AST and produces formatted, human-readable source code.

use std::fmt::Write;
use x3_lang_ast::ast::*;
use x3_lang_ast::{AssetDecl, AtomicTradeDecl, TradeRiskPolicy, TradeStmt};
use x3_lang_common::Spanned;

/// An expression as source text.
///
/// Read by the parser for the one declaration that keeps its expression as text —
/// `invariant <name> { assert <expr> }`, whose body reaches the IR as a string — and
/// by anything that needs the source form of an expression rather than its Rust
/// `Debug`. It used to be `format!("{:?}", expr)`, so the stored text was a Rust
/// value tree and the formatter could not write the body back at all (TICKET-044).
pub fn expression_to_source(expr: &Expression) -> String {
    let mut formatter = X3Formatter::new();
    formatter.format_expression(expr);
    formatter.output
}

/// Whether a clause's expression is the literal zero `from <asset>` fills in
/// when the amount is not stated.
fn is_zero_literal(expression: &Expression) -> bool {
    matches!(expression, Expression::Literal(LiteralExpr::Int { value: 0, .. }))
}

/// Whether a clause's receiver is the `"sender"` an omitted `receiver` fills in.
fn is_sender_default(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Literal(LiteralExpr::String(text)) if text.as_str() == "sender"
    )
}

/// The `chain.ASSET:receiver` a `refund <asset> to <receiver>` clause was
/// folded into, split back into its two parts.
///
/// `None` when the expression is not that shape, which is every refund written
/// as an expression rather than as an asset and a receiver.
fn refund_target(expression: &Expression) -> Option<(String, String)> {
    let Expression::Literal(LiteralExpr::String(text)) = expression else {
        return None;
    };
    let (asset, receiver) = text.as_str().split_once(':')?;
    // An asset carries its chain, so the guard is what distinguishes the folded
    // form from a string that merely contains a colon.
    if !asset.contains('.') {
        return None;
    }
    Some((asset.to_string(), receiver.to_string()))
}

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

    /// Format a program, putting its comments back.
    ///
    /// Each comment is placed before the top-level declaration it precedes — the
    /// finest association available, because the AST holds no comments (the lexer
    /// keeps them, the parser steps over them). A comment *inside* a declaration
    /// therefore moves to that declaration's boundary rather than staying on the
    /// line it was written on: moving is better than deleting, which is what this
    /// used to do, but the caller is told so it can review.
    pub fn format_program_with_comments(
        &mut self,
        program: &Program,
        comments: &[crate::parser::SourceComment],
    ) -> String {
        self.output.clear();
        self.indent_level = 0;
        let mut next = 0usize;
        for item in &program.items {
            let boundary = item.span.start.as_usize();
            while next < comments.len() && comments[next].start < boundary {
                self.write_comment(&comments[next].text);
                next += 1;
            }
            self.format_item(&item.node);
            self.output.push('\n');
        }
        // What remains sits after the last declaration.
        while next < comments.len() {
            self.write_comment(&comments[next].text);
            next += 1;
        }
        self.output.clone()
    }

    /// A comment's own line, at the current indent.
    fn write_comment(&mut self, text: &str) {
        for (index, line) in text.lines().enumerate() {
            if index > 0 {
                self.output.push('\n');
            }
            self.write_indent();
            self.write(line.trim_end());
        }
        self.output.push('\n');
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
            Item::AtomicChoice(c) => self.format_atomic_choice(c),
            Item::VenueDecl(v) => self.format_venue_decl(v),
            Item::ParallelDecl(p) => self.format_parallel_decl(p),
            Item::ObjectiveDecl(o) => self.format_objective_decl(o),
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
            Item::AtomicHedge(hedge) => self.format_atomic_hedge(hedge),
            Item::AtomicLiquidation(liquidation) => self.format_atomic_liquidation(liquidation),
            Item::Rebalance(rebalance) => self.format_rebalance(rebalance),
            Item::Netting(netting) => self.format_netting(netting),
            Item::ProofsRequired(p) => self.format_proofs_required(p),
            Item::VmTarget(t) => self.format_vm_target(t),
            Item::AssetDecl(decl) => self.format_asset_decl(decl),
            Item::TradeRiskPolicy(policy) => self.format_trade_risk_policy(policy),
            Item::AtomicTrade(decl) => self.format_atomic_trade(decl),
        }
    }

    fn format_asset_decl(&mut self, decl: &AssetDecl) {
        self.write("asset ");
        self.write(decl.name.as_str());
        self.write(" = ");
        self.write(decl.asset.vm_family.as_str());
        self.write(".");
        self.write(decl.asset.chain.as_str());
        self.write(".");
        self.write(decl.asset.canonical_id.as_str());
        self.write(" {\n");
        self.indent();
        self.write_indent();
        self.write("decimals: ");
        self.write(&decl.asset.decimals.to_string());
        self.write("\n");
        self.dedent();
        self.write("}\n");
    }

    fn format_atomic_trade(&mut self, decl: &AtomicTradeDecl) {
        self.write("atomic trade ");
        self.write(decl.name.as_str());
        self.write(" using ");
        self.write(decl.risk_policy.as_str());
        self.write(" {\n");
        self.indent();
        for stmt in &decl.body {
            self.format_trade_stmt(stmt);
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_trade_risk_policy(&mut self, policy: &TradeRiskPolicy) {
        self.write("risk policy ");
        self.write(policy.name.as_str());
        self.write(" {\n");
        self.indent();
        self.write_indent();
        self.write("max_slippage: ");
        self.write(&policy.max_slippage_bps.to_string());
        self.write(" bps\n");
        self.write_indent();
        self.write("max_gas: ");
        self.format_expression(&policy.max_gas.value);
        self.write(" ");
        self.write(policy.max_gas.asset.as_str());
        self.write("\n");
        self.write_indent();
        self.write("max_flash_fee: ");
        self.write(&policy.max_flash_fee_bps.to_string());
        self.write(" bps\n");
        self.write_indent();
        self.write("deadline: ");
        self.format_expression(&policy.deadline);
        self.write("\n");
        self.write_indent();
        self.write("require_private_submission: ");
        self.write(if policy.require_private_submission {
            "true"
        } else {
            "false"
        });
        self.write("\n");
        if let Some(min_profit) = &policy.min_profit {
            self.write_indent();
            self.write("min_profit: ");
            self.format_expression(&min_profit.value);
            self.write(" ");
            self.write(min_profit.asset.as_str());
            self.write("\n");
        }
        if let Some(deviation_bps) = policy.max_oracle_deviation_bps {
            self.write_indent();
            self.write("max_oracle_deviation: ");
            self.write(&deviation_bps.to_string());
            self.write(" bps\n");
        }
        if let Some(max_cumulative_loss) = &policy.max_cumulative_loss {
            self.write_indent();
            self.write("max_cumulative_loss: ");
            self.format_expression(&max_cumulative_loss.value);
            self.write(" ");
            self.write(max_cumulative_loss.asset.as_str());
            self.write("\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_trade_stmt(&mut self, stmt: &TradeStmt) {
        self.write_indent();
        match stmt {
            TradeStmt::Borrow { amount, provider, debt } => {
                self.write("borrow ");
                self.format_expression(&amount.value);
                self.write(" ");
                self.write(amount.asset.as_str());
                self.write(" from ");
                self.write(provider.as_str());
                self.write(" as ");
                self.write(debt.0.as_str());
            }
            TradeStmt::Swap {
                binding,
                input,
                from_asset,
                to_asset,
                venue,
                min_output,
            } => {
                self.write("let ");
                self.write(binding.as_str());
                self.write(" = swap ");
                self.format_expression(&input.value);
                self.write(" ");
                self.write(from_asset.as_str());
                self.write(" -> ");
                self.write(to_asset.as_str());
                self.write(" via ");
                self.write(venue.as_str());
                self.write(" min_out ");
                self.format_expression(&min_output.value);
                self.write(" ");
                self.write(min_output.asset.as_str());
            }
            TradeStmt::Bridge {
                input,
                from_asset,
                to_asset,
                via,
                receiver,
            } => {
                self.write("bridge ");
                self.format_expression(&input.value);
                self.write(" ");
                self.write(from_asset.as_str());
                self.write(" -> ");
                self.write(to_asset.as_str());
                self.write(" via ");
                self.write(via.as_str());
                self.write(" to ");
                self.format_expression(receiver);
            }
            TradeStmt::Repay { debt } => {
                self.write("repay ");
                self.write(debt.0.as_str());
            }
            TradeStmt::RequireMinNetProfit { amount } => {
                self.write("require net_profit >= ");
                self.format_expression(&amount.value);
                self.write(" ");
                self.write(amount.asset.as_str());
            }
            TradeStmt::RequireAllDebtsRepaid => {
                self.write("require all_debts_repaid");
            }
            TradeStmt::AssertInvariant { kind } => {
                self.write("invariant ");
                self.write(kind.as_str());
            }
            TradeStmt::EmitReceipt => {
                self.write("emit receipt");
            }
        }
        self.write("\n");
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
        // Every clause the parser reads has to be written back. Emitting only
        // the statement body turned a swap with an amount, a hashlock and two
        // deadlines into a swap with none of them — which still parsed, and so
        // passed any test that only asked whether the output was valid.
        if let Some(amount) = &a.amount {
            self.write_indent();
            self.write("amount ");
            self.format_expression(amount);
            self.write("\n");
        }
        if let Some(receiver) = &a.receiver {
            self.write_indent();
            self.write("receiver ");
            self.format_expression(receiver);
            self.write("\n");
        }
        if let Some(hashlock) = &a.hashlock {
            self.write_indent();
            self.write("hashlock ");
            self.write(hashlock.hash_fn.as_str());
            self.write("(");
            self.format_expression(&hashlock.secret);
            self.write(")\n");
        }
        if let Some(timeout) = &a.timeout_source {
            self.write_indent();
            self.write("timeout source ");
            self.format_expression(timeout);
            self.write("\n");
        }
        if let Some(timeout) = &a.timeout_destination {
            self.write_indent();
            self.write("timeout destination ");
            self.format_expression(timeout);
            self.write("\n");
        }
        for guard in &a.requires {
            self.write_indent();
            self.write("require ");
            self.format_require_guard(guard);
            self.write("\n");
        }
        if let Some(action) = &a.on_fail {
            self.write_indent();
            self.write("on_fail ");
            self.format_failure_action(action);
            self.write("\n");
        }
        for stmt in &a.body {
            self.format_statement(stmt);
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_objective_decl(&mut self, o: &ObjectiveDecl) {
        self.write("objective ");
        self.write(o.name.as_str());
        self.write(" {\n");
        self.indent();
        self.write_indent();
        self.write(o.metric.direction());
        self.write(" ");
        self.write(o.metric.name());
        self.write("\n");
        self.write_indent();
        self.write("constraints {\n");
        self.indent();
        let c = &o.constraints;
        for (name, value) in [
            ("hops", c.max_hops),
            ("chains", c.max_chains),
            ("execution_time", c.max_execution_time_ms),
            ("fees", c.max_fees_bps),
            ("slippage", c.max_slippage_bps),
            ("finality", c.max_finality_blocks),
        ] {
            if let Some(value) = value {
                self.write_indent();
                self.write(name);
                self.write(" <= ");
                self.write(&value.to_string());
                self.write("\n");
            }
        }
        if let Some(risk) = &c.max_risk {
            self.write_indent();
            self.write("risk <= ");
            match risk {
                RiskBound::Score(score) => self.write(&score.to_string()),
                RiskBound::StrategyPolicy => self.write("strategy.policy"),
            }
            self.write("\n");
        }
        if let Some(capital) = &c.capital {
            self.write_indent();
            self.write("capital <= ");
            self.format_amount_expr(capital);
            self.write("\n");
        }
        if c.private {
            self.write_indent();
            self.write("private\n");
        }
        if c.atomic {
            self.write_indent();
            self.write("atomic\n");
        }
        self.dedent();
        self.write_indent();
        self.write("}\n");
        self.dedent();
        self.write("}\n");
    }

    fn format_parallel_decl(&mut self, p: &ParallelDecl) {
        self.write("parallel ");
        self.write(p.name.as_str());
        self.write(" {\n");
        self.indent();
        for leg in &p.legs {
            self.write_indent();
            self.write("leg ");
            self.write(leg.name.as_str());
            self.write(" {\n");
            self.indent();
            for statement in &leg.body {
                self.format_statement(statement);
            }
            self.dedent();
            self.write_indent();
            self.write("}\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_venue_decl(&mut self, v: &VenueDecl) {
        self.write("venue ");
        self.write(v.name.as_str());
        self.write(" {\n");
        self.indent();
        self.write_indent();
        self.write("kind ");
        self.write(v.kind.as_str());
        self.write("\n");
        self.write_indent();
        self.write("chain ");
        self.write(v.chain.as_str());
        self.write("\n");
        self.write_indent();
        self.write("domain ");
        self.write(v.domain.as_str());
        self.write("\n");
        self.write_indent();
        self.write("asset_in ");
        self.format_asset_ref(&v.asset_in);
        self.write("\n");
        self.write_indent();
        self.write("asset_out ");
        self.format_asset_ref(&v.asset_out);
        self.write("\n");
        for (field, value) in [
            ("fee_bps", v.fee_bps),
            ("slippage_bps", v.slippage_bps),
            ("latency_ms", v.latency_ms),
            ("finality_blocks", v.finality_blocks),
            ("risk", v.risk),
        ] {
            self.write_indent();
            self.write(field);
            self.write(" ");
            self.write(&value.to_string());
            self.write("\n");
        }
        self.write_indent();
        self.write("liquidity ");
        self.write(&v.liquidity.to_string());
        self.write("\n");
        if let Some(proof) = &v.proof {
            self.write_indent();
            self.write("proof ");
            self.write(proof.as_str());
            self.write("\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_atomic_choice(&mut self, c: &AtomicChoiceDecl) {
        self.write("atomic_choice ");
        self.write(c.name.as_str());
        self.write(" {\n");
        self.indent();
        for path in &c.paths {
            self.write("path ");
            self.write(path.name.as_str());
            self.write(" {\n");
            self.indent();
            if !path.hops.is_empty() {
                for (index, hop) in path.hops.iter().enumerate() {
                    if index > 0 {
                        self.write(" -> ");
                    }
                    self.format_asset_ref(hop);
                }
                self.write("\n");
            }
            for stmt in &path.body {
                self.format_statement(stmt);
            }
            if let Some(output) = &path.net_output {
                self.write("net_output ");
                self.format_expression(&output.value);
                self.write(" ");
                self.write(output.asset.as_str());
                self.write("\n");
            }
            self.dedent();
            self.write("}\n");
        }
        self.write("choose ");
        self.write(c.criterion.as_str());
        self.write(";\n");
        self.dedent();
        self.write("}\n");
    }

    fn format_strategy(&mut self, s: &CrossChainStrategy) {
        self.write("strategy ");
        self.write(s.name.as_str());
        self.write(" {\n");
        self.indent();
        // The eight declarations are part of the module, so they are part of
        // what the formatter prints. A formatter that dropped them would turn a
        // checked module into one the verifier refuses, which is the worst
        // possible outcome for a source-level tool.
        for input in &s.inputs {
            self.write_indent();
            self.write("input ");
            self.format_asset_ref(&input.asset);
            if let Some(amount) = &input.amount {
                self.write(" amount ");
                self.format_expression(amount);
            }
            if let Some(max) = &input.max_amount {
                self.write(" max ");
                self.format_expression(max);
            }
            self.write("\n");
        }
        for output in &s.outputs {
            self.write_indent();
            self.write("output ");
            self.format_asset_ref(output);
            self.write("\n");
        }
        let names = |items: Vec<&str>| items.join(", ");
        if !s.effects.is_empty() {
            self.write_indent();
            self.write("effects [");
            self.write(&names(s.effects.iter().map(|effect| effect.as_str()).collect()));
            self.write("]\n");
        }
        if !s.guarantees.is_empty() {
            self.write_indent();
            self.write("guarantees [");
            self.write(&names(s.guarantees.iter().map(|g| g.as_str()).collect()));
            self.write("]\n");
        }
        if !s.permissions.is_empty() {
            self.write_indent();
            self.write("permissions [");
            self.write(&names(
                s.permissions.iter().map(|permission| permission.as_str()).collect(),
            ));
            self.write("]\n");
        }
        if !s.domains.is_empty() {
            self.write_indent();
            self.write("domains [");
            self.write(&names(s.domains.iter().map(|domain| domain.as_str()).collect()));
            self.write("]\n");
        }
        if let Some(risk) = &s.risk {
            self.write_indent();
            self.write("risk { max_slippage_bps ");
            self.write(&risk.max_slippage_bps.to_string());
            self.write(" max_total_fee_bps ");
            self.write(&risk.max_total_fee_bps.to_string());
            self.write(" }\n");
        }
        if let Some(license) = &s.license {
            // The licence and the split are part of the module, so the formatter
            // prints them: dropping them would turn a licensed module into an
            // unlicensed one, which is a materially different artifact.
            self.write_indent();
            self.write("license { creator ");
            self.write(license.creator.as_str());
            self.write(" profit_share ");
            self.write(&format!("{}%", license.profit_share_bps / 100));
            if let Some(executions) = license.executions {
                self.write(" executions ");
                self.write(&executions.to_string());
            }
            if let Some(block) = license.expires_block {
                self.write(" expires_block ");
                self.write(&block.to_string());
            }
            self.write(" }\n");
        }
        if let Some(submission) = &s.submission {
            self.write_indent();
            self.write("submission { private = ");
            self.write(submission.private.as_str());
            self.write(" }\n");
        }
        if let Some(split) = &s.split {
            self.write_indent();
            self.write("split profit {\n");
            self.indent();
            for (recipient, bps) in &split.shares {
                self.write_indent();
                self.write(&format!("{}% -> {}", bps / 100, recipient.as_str()));
                self.write("\n");
            }
            self.dedent();
            self.write_indent();
            self.write("}\n");
        }
        self.write_indent();
        self.write("bounds { ");
        if let Some(max_steps) = &s.max_steps {
            self.write("max_steps ");
            self.format_expression(max_steps);
            self.write(" ");
        }
        if let Some(max_gas) = &s.max_gas {
            self.write("max_gas ");
            self.format_expression(max_gas);
            self.write(" ");
        }
        self.write("}\n");
        self.write_indent();
        self.write("execute {\n");
        self.indent();
        for stmt in &s.body {
            self.format_statement(stmt);
        }
        self.dedent();
        self.write_indent();
        self.write("}\n");
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
            self.format_intent_statement(stmt);
        }
        self.dedent();
        self.write("}\n");
    }

    /// One statement inside an `intent` body, in the intent's own dialect.
    ///
    /// An intent body is not a block of statements: `from`, `to` and `route` are
    /// clauses that *lower* to `lock`, `release` and `atomic`, and the parser
    /// reads the clauses back. Printing the lowered form is what made `x3c fmt`
    /// write text no intent can contain — the parser there accepts `from`, `to`,
    /// `route`, `require`, `timeout`, `on_fail`, `allow`, `use` and `on`, and
    /// nothing else.
    fn format_intent_statement(&mut self, stmt: &Statement) {
        match stmt {
            // `from <chain.ASSET> [amount <n>] [receiver <r>]`. The parser
            // fills in zero and `"sender"` when the clause omits them, so those
            // are the two values that are written only when they were stated.
            Statement::Lock {
                chain,
                asset,
                amount,
                from,
            } => {
                self.write_indent();
                self.write("from ");
                self.write(chain.as_str());
                self.write(".");
                self.write(asset.name.as_str());
                if !is_zero_literal(amount) {
                    self.write(" amount ");
                    self.format_expression(amount);
                }
                if !is_sender_default(from) {
                    self.write(" receiver ");
                    self.format_expression(from);
                }
                self.write("\n");
            }
            Statement::Release { chain, asset, to } => {
                self.write_indent();
                self.write("to ");
                self.write(chain.as_str());
                self.write(".");
                self.write(asset.name.as_str());
                if !is_sender_default(to) {
                    self.write(" receiver ");
                    self.format_expression(to);
                }
                self.write("\n");
            }
            Statement::Atomic(atomic) if atomic.meta.is_none() => {
                self.write_indent();
                self.write("route {\n");
                self.indent();
                for step in &atomic.body.stmts {
                    self.format_statement(step);
                }
                self.dedent();
                self.write_indent();
                self.write("}\n");
            }
            // `timeout <n> [refund <asset> to <receiver>]`. The duration is
            // stored in blocks, which is what a bare number means.
            Statement::OnTimeout { duration, action } => {
                self.write_indent();
                self.write("timeout ");
                self.format_expression(duration);
                if !matches!(action, FailureAction::Rollback) {
                    self.write(" ");
                    self.format_failure_action(action);
                }
                self.write("\n");
            }
            // `use <target> <config>` and `on <event> <action>` parse into a
            // call so that lowering has one shape to walk; the clause is what the
            // intent surface accepts, so the call is written back as one.
            Statement::Expr(Expression::Call { callee, args })
                if matches!(&**callee, Expression::Ident(name) if name.as_str() == "use" || name.as_str() == "on")
                    && args.len() == 2 =>
            {
                let Expression::Ident(clause) = &**callee else {
                    unreachable!("matched above")
                };
                self.write_indent();
                self.write(clause.as_str());
                self.write(" ");
                self.format_expression(&args[0]);
                self.write(" ");
                self.format_expression(&args[1]);
                self.write("\n");
            }
            Statement::Require(guard) => {
                self.write_indent();
                self.write("require ");
                self.format_require_guard(guard);
                self.write("\n");
            }
            Statement::OnFail(action) => {
                self.write_indent();
                self.write("on_fail ");
                self.format_failure_action(action);
                self.write("\n");
            }
            Statement::Allow { feature } => {
                self.write_indent();
                self.write("allow ");
                self.write(feature.as_str());
                self.write("\n");
            }
            other => self.format_statement(other),
        }
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
        // `vm { chain <c> adapter <a> finality <f> }` — the parser reads clauses,
        // not a chain name in the header followed by a field list.
        self.write("vm {\n");
        self.indent();
        self.write_indent();
        self.write("chain ");
        self.write(v.chain.as_str());
        self.write("\n");
        self.write_indent();
        self.write("adapter ");
        self.write(v.adapter.as_str());
        self.write("\n");
        if let Some(f) = &v.finality {
            self.write_indent();
            self.write("finality ");
            self.write(f.as_str());
            self.write("\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_solver_market(&mut self, m: &SolverMarket) {
        self.write("solver_market {\n");
        self.indent();
        self.write_indent();
        self.write("mode ");
        self.write(m.mode.as_str());
        self.write("\n");
        self.write_indent();
        self.write("min_reputation ");
        self.write(&m.min_reputation.to_string());
        self.write("\n");
        if let Some(bond) = &m.bond {
            self.write_indent();
            self.write("bond ");
            self.format_amount_expr(bond);
            self.write("\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_relayer_swarm(&mut self, r: &RelayerSwarm) {
        self.write("relayers {\n");
        self.indent();
        self.write_indent();
        self.write("quorum_numerator ");
        self.write(&r.quorum_numerator.to_string());
        self.write("\n");
        self.write_indent();
        self.write("quorum_denominator ");
        self.write(&r.quorum_denominator.to_string());
        self.write("\n");
        self.write_indent();
        self.write("relayers [");
        for (i, rel) in r.relayers.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            self.write(rel.as_str());
        }
        self.write("]\n");
        self.dedent();
        self.write("}\n");
    }

    fn format_rpc_quorum(&mut self, q: &RpcQuorum) {
        // `rpc_quorum { source <chain> require <n>_of_<m> reject_on [ … ] }` —
        // the chain is a clause inside the block, not a name in the header.
        self.write("rpc_quorum {\n");
        self.indent();
        self.write_indent();
        self.write("source ");
        self.write(q.source.as_str());
        self.write("\n");
        // The two field names rather than `require <n>_of_<m>`: `require` is a
        // keyword, and the shorthand arm that reads it was unreachable until it
        // was given a token to match, so the explicit form is the one every
        // program that parses today was written in.
        self.write_indent();
        self.write("require_numerator ");
        self.write(&q.require_numerator.to_string());
        self.write("\n");
        self.write_indent();
        self.write("require_denominator ");
        self.write(&q.require_denominator.to_string());
        self.write("\n");
        if !q.reject_on.is_empty() {
            self.write_indent();
            self.write("reject_on [");
            for (index, reason) in q.reject_on.iter().enumerate() {
                if index > 0 {
                    self.write(", ");
                }
                self.write(reason.as_str());
            }
            self.write("]\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_risk_policy(&mut self, p: &RiskPolicy) {
        // Clauses, one per line: the parser reads `max_slippage <n>`, and a
        // colon is not a token either clause can step over.
        self.write("risk_policy {\n");
        self.indent();
        self.write_indent();
        self.write("max_slippage ");
        self.write(&p.max_slippage.to_string());
        self.write("\n");
        if let Some(position) = &p.max_position {
            self.write_indent();
            self.write("max_position ");
            self.write(&position.to_string());
            self.write("\n");
        }
        if let Some(score) = &p.min_route_score {
            self.write_indent();
            self.write("min_route_score ");
            self.write(&score.to_string());
            self.write("\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_privacy_block(&mut self, p: &PrivacyBlock) {
        self.write("privacy {\n");
        self.indent();
        self.write_indent();
        self.write("hide_route_until_commit ");
        self.write(if p.hide_route_until_commit { "true" } else { "false" });
        self.write("\n");
        self.write_indent();
        self.write("reveal_on ");
        self.write(p.reveal_on.as_str());
        self.write("\n");
        self.write_indent();
        self.write("encrypted ");
        self.write(if p.encrypted { "true" } else { "false" });
        self.write("\n");
        self.dedent();
        self.write("}\n");
    }

    fn format_invariant(&mut self, i: &InvariantDecl) {
        // The parser stores the name as the assertion when the declaration has
        // no body, so an `invariant <name>` written back with a body would
        // assert the name against the text of the name.
        self.write("invariant ");
        self.write(i.name.as_str());
        if i.assert_expr.as_str() != i.name.as_str() {
            self.write(" { assert ");
            self.write(i.assert_expr.as_str());
            self.write(" }");
        }
        self.write("\n");
    }

    fn format_error(&mut self, e: &ErrorDecl) {
        // `error <name>` is a whole item with no terminator; a trailing `;` is
        // the next top-level token, and the item loop has no case for it.
        self.write("error ");
        self.write(e.name.as_str());
        self.write("\n");
    }

    /// `rebalance <name> { <ASSET> = <pct>; … minimize { <metric>; … } atomic; }`
    fn format_rebalance(&mut self, rebalance: &RebalanceDecl) {
        self.write("rebalance ");
        self.write(rebalance.name.as_str());
        self.write(" {\n");
        self.indent();
        for (asset, percent) in &rebalance.weights {
            self.write_indent();
            self.format_asset_ref(asset);
            self.write(&format!(" = {percent}%;\n"));
        }
        self.write_indent();
        self.write("minimize {\n");
        self.indent();
        for metric in &rebalance.minimize {
            self.write_indent();
            self.write(&format!("{};\n", metric.name()));
        }
        self.dedent();
        self.write_indent();
        self.write("}\n");
        self.write_indent();
        self.write("atomic;\n");
        self.dedent();
        self.write("}\n");
    }

    /// `netting <name> { consent <party>; <debtor> owes <n> <chain.ASSET> to <creditor>; }`
    fn format_netting(&mut self, netting: &NettingDecl) {
        self.write("netting ");
        self.write(netting.name.as_str());
        self.write(" {\n");
        self.indent();
        for party in &netting.consent {
            self.write_indent();
            self.write(&format!("consent {};\n", party.as_str()));
        }
        for obligation in &netting.obligations {
            self.write_indent();
            self.write(&format!("{} owes {} ", obligation.debtor.as_str(), obligation.amount));
            self.format_asset_ref(&obligation.asset);
            self.write(&format!(" to {};\n", obligation.creditor.as_str()));
        }
        self.dedent();
        self.write("}\n");
    }

    /// `atomic_liquidation { liquidate …; receive …; swap …; repay …; require net_profit …; }`
    fn format_atomic_liquidation(&mut self, liquidation: &AtomicLiquidationDecl) {
        self.write("atomic_liquidation {\n");
        self.indent();
        self.write_indent();
        self.write(&format!("liquidate {} ", liquidation.capital.0));
        self.format_asset_ref(&liquidation.capital.1);
        self.write(&format!(" of {}\n", liquidation.position.as_str()));
        self.write_indent();
        self.write(&format!("receive {} ", liquidation.collateral.0));
        self.format_asset_ref(&liquidation.collateral.1);
        self.write(" collateral\n");
        self.write_indent();
        self.write(&format!("swap {} ", liquidation.swap.amount));
        self.format_asset_ref(&liquidation.swap.from);
        self.write(" -> ");
        self.format_asset_ref(&liquidation.swap.to);
        self.write(&format!(" min_output {}\n", liquidation.swap.min_output));
        self.write_indent();
        self.write(&format!("repay {} ", liquidation.repaid.0));
        self.format_asset_ref(&liquidation.repaid.1);
        self.write("\n");
        if let Some((floor, asset)) = &liquidation.profit_floor {
            self.write_indent();
            self.write(&format!("require net_profit >= {floor} "));
            self.format_asset_ref(asset);
            self.write("\n");
        }
        self.dedent();
        self.write("}\n");
    }

    /// `atomic_hedge { buy … spot; short … perp; require delta <= <pct>; }`
    fn format_atomic_hedge(&mut self, hedge: &AtomicHedgeDecl) {
        self.write("atomic_hedge {\n");
        self.indent();
        for leg in &hedge.legs {
            self.write_indent();
            self.write(match leg.side {
                HedgeSide::Long => "buy ",
                HedgeSide::Short => "short ",
            });
            match leg.quantity {
                HedgeQuantity::Amount(amount) => self.write(&amount.to_string()),
                HedgeQuantity::Equivalent => self.write("equivalent"),
            }
            self.write(" ");
            self.format_asset_ref(&leg.asset);
            self.write(match leg.venue {
                HedgeVenue::Spot => " spot\n",
                HedgeVenue::Perp => " perp\n",
            });
        }
        if let Some(bound) = hedge.delta_bound_bps {
            self.write_indent();
            self.write(&format!("require delta <= {bound} bps;\n"));
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_finality_policy(&mut self, f: &FinalityPolicy) {
        self.write("finality_policy ");
        self.write(f.mode.as_str());
        self.write(" {\n");
        self.indent();
        self.write_indent();
        self.write("chain ");
        self.write(f.chain.as_str());
        self.write("\n");
        self.write_indent();
        self.write("requirement ");
        self.write(f.requirement.as_str());
        self.write("\n");
        if let Some(blocks) = f.blocks {
            self.write_indent();
            self.write(&format!("blocks {blocks}\n"));
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_proofs_required(&mut self, p: &ProofsRequired) {
        // One name per line, no commas: the list is a block of names.
        self.write("proofs required {\n");
        self.indent();
        for proof in &p.proofs {
            self.write_indent();
            self.write(proof.as_str());
            self.write("\n");
        }
        self.dedent();
        self.write("}\n");
    }

    fn format_vm_target(&mut self, t: &VmTarget) {
        self.write("target ");
        self.write(t.vm.as_str());
        self.write(" {\n");
        self.indent();
        self.write_indent();
        self.write("adapter ");
        self.write(t.adapter.as_str());
        self.write("\n");
        if let Some(c) = &t.contract {
            self.write_indent();
            self.write("contract ");
            self.write(c.as_str());
            self.write("\n");
        }
        self.dedent();
        self.write("}\n");
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
                amount,
                min_output,
                dex,
            } => {
                self.write_indent();
                // `swap <venue> <from> -> <to> …` — the parser reads the venue
                // immediately after `swap`, so writing it first would make the
                // venue the verb's object and the asset the venue.
                self.write("swap ");
                if let Some(Expression::Literal(LiteralExpr::String(s))) = dex {
                    self.write(s.as_str());
                    self.write(" ");
                }
                self.format_asset_ref(from);
                self.write(" -> ");
                self.format_asset_ref(to);
                if let Some(amt) = amount {
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
                self.format_require_guard(guard);
                self.write(";\n");
            }
            Statement::Allow { feature } => {
                self.write_indent();
                self.write("allow ");
                self.write(feature.as_str());
                self.write(";\n");
            }
            Statement::OnFail(action) => {
                self.write_indent();
                self.write("on_fail ");
                self.format_failure_action(action);
                self.write(";\n");
            }
            Statement::RouteFallback { replacements, requires } => {
                self.write_indent();
                self.write("fallback {\n");
                self.indent();
                for replacement in replacements {
                    self.write_indent();
                    self.write("replace with ");
                    self.write(replacement.venue.as_str());
                    if let Some(min_output) = &replacement.min_output {
                        self.write(" min_output ");
                        self.format_expression(min_output);
                    }
                    self.write(";\n");
                }
                for guard in requires {
                    self.write_indent();
                    self.write("require ");
                    // The comparison is part of the guard. Writing only the kind
                    // and the value turned `require slippage <= 7` into
                    // `require slippage 7`, which the fallback check reads as
                    // "not a ceiling" and refuses.
                    self.format_require_guard(guard);
                    self.write(";\n");
                }
                self.dedent();
                self.write_indent();
                self.write("}\n");
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
                // `{op}` and not `{op:?}`: `BinOp` implements `Display` with the
                // operator as the language writes it, and `Debug` gives the variant's
                // name — `Ge`, `Plus` — which is not a program. A formatter that
                // writes one produces text the parser refuses.
                self.format_expression(lhs);
                self.write(&format!(" {op} "));
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
            LiteralExpr::Duration { value, unit } => {
                // `180s`, not `180`: a bare number is a count of *blocks*, so
                // dropping the unit would change the deadline by the block time.
                self.write(&value.to_string());
                self.write(duration_unit_suffix(*unit));
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

    /// `<amount> <ASSET>` — an amount and the asset it is denominated in.
    fn format_amount_expr(&mut self, amount: &x3_lang_ast::trading::AmountExpr) {
        self.format_expression(&amount.value);
        self.write(" ");
        self.write(amount.asset.as_str());
    }

    fn format_asset_ref(&mut self, asset: &AssetRef) {
        self.write(asset.chain.as_str());
        self.write(".");
        self.write(asset.name.as_str());
    }

    /// `require`'s contents: `kind[.subject] [comparison] value`.
    ///
    /// The caller writes the keyword and the terminator, so the same shape is
    /// used by a statement, an `atomic swap` clause and a choice path — a guard
    /// written by one and read by the other cannot drift.
    fn format_require_guard(&mut self, guard: &RequireGuard) {
        self.format_require_kind(&guard.kind);
        if let Some(subject) = &guard.subject {
            // The dot form when a comparison follows, because
            // `require finality arbitrum >= 32` and `require finality.arbitrum >= 32`
            // are the same guard but only one reads as a property of a chain.
            self.write(if guard.comparison.is_some() { "." } else { " " });
            self.write(subject.as_str());
        }
        if let Some(comparison) = guard.comparison {
            self.write(" ");
            self.write(comparison.as_str());
        }
        // A guard with no value names a property, and its subject is what it
        // names: `require canonical_supply USDC` writes back as itself, which
        // the parser reads as the same guard.
        if let Some(value) = &guard.value {
            self.write(" ");
            self.format_expression(value);
        }
    }

    /// What `on_fail` does next.
    fn format_failure_action(&mut self, action: &FailureAction) {
        match action {
            FailureAction::Rollback => self.write("rollback"),
            FailureAction::Halt => self.write("halt"),
            FailureAction::Quarantine => self.write("quarantine"),
            FailureAction::Refund(expression) => {
                self.write("refund ");
                // Two producers, two shapes. An intent's `on_fail refund
                // <chain.ASSET> to <receiver>` clause folds the asset and the
                // receiver into one string; the generic action keeps whatever
                // expression it was handed. Writing the folded form back as an
                // expression prints a quoted string that the clause parser reads
                // as *not* an asset, and the refund silently becomes a rollback.
                match refund_target(expression) {
                    Some((asset, receiver)) => {
                        self.write(&asset);
                        if receiver != "sender" {
                            self.write(" to ");
                            self.write(&receiver);
                        }
                    }
                    None => self.format_expression(expression),
                }
            }
        }
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

/// The suffix a duration unit is written with.
fn duration_unit_suffix(unit: x3_lang_common::DurationUnit) -> &'static str {
    use x3_lang_common::DurationUnit;
    match unit {
        DurationUnit::Nanoseconds => "ns",
        DurationUnit::Microseconds => "us",
        DurationUnit::Milliseconds => "ms",
        DurationUnit::Seconds => "s",
        DurationUnit::Minutes => "m",
        DurationUnit::Hours => "h",
        DurationUnit::Days => "d",
    }
}
