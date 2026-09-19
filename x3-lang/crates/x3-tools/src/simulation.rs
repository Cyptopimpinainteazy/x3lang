//! Deterministic strategy simulation — spec PHASE 54.
//!
//! The phase asks for `x3c simulate arb.x3 --state snapshot.json --explain` and shows
//! a report of the shape "Route selected / Capital / Gross / Fees / Net / Minimum
//! required / Result". This module is the state and the arithmetic behind it; the
//! command that reads it is in `bin/x3c.rs`.
//!
//! ## Why a snapshot and not a benchmark
//!
//! A dry run has no prices. The compiler's economic guards are therefore *measured*
//! guards (`RequireKind::Measured`, PHASE 37/38): the artifact refuses with
//! `X3_GUARD_UNMEASURED` unless a host states what the market did. On the command line
//! that is `--measured-profit-bps`, which is one number for a whole trade. PHASE 54
//! wants the *accounting* — capital, gross, fees, net — so the snapshot states the
//! observations separately and this module derives the rest. The two are the same fact
//! stated two ways, so stating both is refused rather than reconciled.
//!
//! ## What is derived rather than accepted
//!
//! - **Net** is `gross − capital − fees`, never a stated figure. A snapshot that
//!   stated its own net could disagree with its own components.
//! - **Net in basis points** is [`Ratio::of`] of capital into net, read with
//!   [`Ratio::to_bps`]. Not a division by a literal ten thousand: PHASE 43 put that
//!   arithmetic in one place.
//! - **Minimum required**, in the asset, is the artifact's own floor in basis points
//!   applied to the capital this snapshot states ([`Bps::of_floor`]). The floor is
//!   read from the artifact, never from the snapshot — a caller may not tell the
//!   report what to compare against.
//!
//! ## What is checked rather than printed
//!
//! The snapshot names the venues the host observed the trade use. Where the artifact
//! declares an approved venue list (`RouteFallback`, lowered by PHASE 37's `arb`), a
//! venue outside every approved list is refused with the list. Where the artifact
//! declares none, the report says the venues were **not** checked rather than
//! implying they were.
//!
//! ## Determinism (PHASE 42)
//!
//! The same artifact and the same snapshot produce a byte-identical report. Nothing
//! here reads a clock, a random source, or a hash map: the fields are `Vec`s in
//! document order, every operation is integer, every refusal names the figure that
//! caused it, and the report is assembled in one `String`. `render` is a pure
//! function of the outcome.

use serde::Deserialize;
use std::fmt;

use x3_lang_common::fixed::{Bps, Ratio, RoundingMode};

/// The version of the snapshot schema this build reads.
///
/// A snapshot from another version is refused rather than read for the fields it
/// happens to share: the fields are the accounting, and a partial read is a partial
/// account.
pub const SNAPSHOT_VERSION: u32 = 1;

/// Everything the host observed about one trade, as a simulation input.
///
/// `deny_unknown_fields` is deliberate. A misspelled field is a state the host meant
/// to state and this build cannot see; ignoring it would simulate a *different*
/// market from the one described and report on it with confidence.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationSnapshot {
    pub version: u32,
    pub route: RouteObservation,
    pub capital: Amount,
    pub gross: Amount,
    pub fees: Amount,
    /// The slippage the host observed, in basis points.
    ///
    /// Optional because a program need not state a slippage ceiling, and a value
    /// nothing compares should not be required. It *is* required when the artifact
    /// carries a measured slippage ceiling — see [`SimulationError::SlippageUnstated`].
    #[serde(default)]
    pub slippage_bps: Option<u32>,
}

/// The route the host observed the trade take.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteObservation {
    /// The chains crossed, in the order they were crossed.
    pub chains: Vec<String>,
    /// The venues traded on, in the order they were used. Checked against the
    /// artifact's approved venue lists; see the module documentation.
    pub venues: Vec<String>,
}

/// An amount of one asset. Absolute, in the asset's base units.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Amount {
    pub asset: String,
    pub amount: u128,
}

/// Why a snapshot could not be simulated.
///
/// Every variant names the figures that produced it. A refusal a reader cannot act on
/// is a refusal that gets worked around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimulationError {
    UnsupportedVersion {
        found: u32,
        supported: u32,
    },
    RouteEmpty,
    /// The three amounts are not all of one asset, so netting them would be a claim
    /// about a price this tool does not have.
    AssetMismatch {
        field: &'static str,
        asset: String,
        expected: String,
    },
    /// `gross < capital + fees`: the trade returned less than it committed.
    Underwater {
        capital: u128,
        fees: u128,
        gross: u128,
    },
    /// A profit in basis points is a fraction of the capital, and there is none.
    ZeroCapital,
    /// The capital is too large for the bps arithmetic to represent.
    NotRepresentable {
        capital: u128,
        net: u128,
    },
    /// The artifact states a slippage ceiling and the snapshot states no slippage.
    SlippageUnstated {
        ceiling_bps: u32,
    },
    /// The artifact states a profit floor and the capital is too large for the floor
    /// to be expressed in the asset.
    FloorNotRepresentable {
        floor_bps: u32,
        capital: u128,
    },
    /// A venue the host observed is in no approved list the artifact declares.
    UnapprovedVenue {
        venue: String,
        approved: Vec<String>,
    },
}

impl fmt::Display for SimulationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                "the snapshot states schema version {found}; this build reads version {supported} \
                 — the fields are the accounting, so a version it does not know is refused rather \
                 than read for the fields the two happen to share"
            ),
            Self::RouteEmpty => write!(
                f,
                "the snapshot states no route: a simulation reports the route that was taken, and a \
                 route of nothing is not one"
            ),
            Self::AssetMismatch { field, asset, expected } => write!(
                f,
                "the snapshot's {field} is in '{asset}' and its capital is in '{expected}': net is \
                 gross less capital less fees, and subtracting across assets would be a claim \
                 about a price this tool does not have"
            ),
            Self::Underwater { capital, fees, gross } => write!(
                f,
                "the snapshot states gross {gross}, against capital {capital} and fees {fees} — \
                 the trade returned less than the {capital} it committed plus the {fees} it paid, \
                 so it has no net and this is a loss rather than a profit to compare"
            ),
            Self::ZeroCapital => write!(
                f,
                "the snapshot states zero capital: a profit in basis points is a fraction of the \
                 capital, and a fraction of nothing is not a smaller profit, it is undefined"
            ),
            Self::NotRepresentable { capital, net } => write!(
                f,
                "the snapshot's capital {capital} and net {net} are too large to express as a \
                 ratio of one another in this arithmetic"
            ),
            Self::SlippageUnstated { ceiling_bps } => write!(
                f,
                "the artifact states a slippage ceiling of {ceiling_bps}bps and the snapshot states \
                 no slippage for the run to be judged against; the ceiling would otherwise be \
                 compared against a number this tool invented"
            ),
            Self::FloorNotRepresentable { floor_bps, capital } => write!(
                f,
                "the artifact's profit floor of {floor_bps}bps applied to a capital of {capital} \
                 does not fit the amount type, so the minimum required cannot be stated"
            ),
            Self::UnapprovedVenue { venue, approved } => write!(
                f,
                "the snapshot says the trade used venue '{venue}', which is in none of the venue \
                 lists the artifact approved ({}); a route through a venue the compiler did not \
                 approve is not the route the artifact describes",
                approved.join(", ")
            ),
        }
    }
}

/// What the artifact's own floors are, read from the compiled bytecode.
///
/// The simulation never takes these from the snapshot: the thing being tested is the
/// artifact's requirement, and a caller who could state the requirement could make any
/// run pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactFloors {
    /// The `profit >= <n>bps` floor the artifact states, if it states one.
    pub profit_floor_bps: Option<u32>,
    /// The `slippage <= <n>bps` ceiling the artifact states, if it states one.
    pub slippage_ceiling_bps: Option<u32>,
    /// Every venue the artifact approved, flattened across its approved lists. Empty
    /// means the artifact declares no venue restriction at all.
    pub approved_venues: Vec<String>,
}

/// The result of comparing a run's net against the artifact's floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The artifact states a floor and the net cleared it.
    Pass,
    /// The artifact states a floor and the net did not clear it.
    BelowFloor { net_bps: u32, required_bps: u32 },
    /// The artifact states no profit floor, so there is nothing for the net to pass.
    /// This is a *finding*, not a pass: a strategy with no floor can lose money and
    /// still settle.
    NoFloorStated,
}

impl Verdict {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::BelowFloor { .. } => "FAIL",
            Self::NoFloorStated => "NO FLOOR STATED",
        }
    }
}

/// The accounting a simulation reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomicOutcome {
    pub chains: Vec<String>,
    pub venues: Vec<String>,
    /// Whether the observed venues were checked against the artifact's approved lists.
    pub venues_checked: bool,
    pub asset: String,
    pub capital: u128,
    pub gross: u128,
    pub fees: u128,
    /// `gross − capital − fees`. Derived, never accepted.
    pub net: u128,
    /// `net` as basis points of `capital`.
    pub net_bps: u32,
    /// The artifact's floor, in basis points, if it states one.
    pub floor_bps: Option<u32>,
    /// That floor applied to this capital — the figure the spec's report calls
    /// "Minimum required".
    pub minimum_required: Option<u128>,
    /// The slippage the run was judged against, when the artifact states a ceiling.
    pub slippage_bps: Option<u32>,
    /// The ceiling that slippage was compared with, when the artifact states one.
    pub slippage_ceiling_bps: Option<u32>,
    pub verdict: Verdict,
}

impl SimulationSnapshot {
    /// Read a snapshot from a file, or say why it is not one.
    pub fn read(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|error| format!("read {path:?}: {error}"))?;
        Self::from_json(&text)
    }

    /// Parse a snapshot, refusing a schema version this build does not read.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let snapshot: Self = serde_json::from_str(text).map_err(|error| {
            format!(
                "the state snapshot is not a schema-{SNAPSHOT_VERSION} observation: {error} \
                 (required: version, route{{chains,venues}}, capital{{asset,amount}}, \
                 gross{{asset,amount}}, fees{{asset,amount}}, optional slippage_bps)"
            )
        })?;
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(SimulationError::UnsupportedVersion {
                found: snapshot.version,
                supported: SNAPSHOT_VERSION,
            }
            .to_string());
        }
        Ok(snapshot)
    }

    /// The profit this run realised, in basis points — what the VM's measured profit
    /// guard is judged against.
    ///
    /// The accounting alone. It is available *before* the artifact is executed, when
    /// the approved venue list is not yet known, and it is the only part of
    /// [`SimulationSnapshot::evaluate`] that has to be: the measured profit is an
    /// input to the run rather than a conclusion of it.
    pub fn measured_profit_bps(&self) -> Result<u32, SimulationError> {
        Ok(self.accounting()?.1)
    }

    /// Do the accounting, check the route against the artifact's approvals, and compare
    /// the result against the artifact's own floor.
    pub fn evaluate(&self, floors: &ArtifactFloors) -> Result<EconomicOutcome, SimulationError> {
        let (net, net_bps) = self.accounting()?;
        let slippage_bps = self.measured_slippage_bps(floors)?;

        // The floor is the artifact's, and so is the asset figure it implies.
        let minimum_required = match floors.profit_floor_bps {
            Some(floor_bps) => Some(Bps::from_raw(floor_bps).of_floor(self.capital.amount).ok_or(
                SimulationError::FloorNotRepresentable {
                    floor_bps,
                    capital: self.capital.amount,
                },
            )?),
            None => None,
        };

        let verdict = match floors.profit_floor_bps {
            Some(required_bps) if net_bps >= required_bps => Verdict::Pass,
            Some(required_bps) => Verdict::BelowFloor { net_bps, required_bps },
            None => Verdict::NoFloorStated,
        };

        // The observed route is a claim about the market; the artifact's approved lists
        // are the compiler's decision about which venues may be used. Where the artifact
        // states one, the claim is checked against it.
        let venues_checked = !floors.approved_venues.is_empty();
        if venues_checked {
            for venue in &self.route.venues {
                if !floors.approved_venues.contains(venue) {
                    return Err(SimulationError::UnapprovedVenue {
                        venue: venue.clone(),
                        approved: floors.approved_venues.clone(),
                    });
                }
            }
        }

        Ok(EconomicOutcome {
            chains: self.route.chains.clone(),
            venues: self.route.venues.clone(),
            venues_checked,
            asset: self.capital.asset.clone(),
            capital: self.capital.amount,
            gross: self.gross.amount,
            fees: self.fees.amount,
            net,
            net_bps,
            floor_bps: floors.profit_floor_bps,
            minimum_required,
            slippage_bps,
            slippage_ceiling_bps: floors.slippage_ceiling_bps,
            verdict,
        })
    }

    /// The slippage this run is judged against, or `None` when the artifact states no
    /// ceiling for it to be compared with.
    ///
    /// The single place the ceiling and the observation are put together, so the command
    /// and the report cannot disagree about whether the snapshot was sufficient. A
    /// ceiling with nothing to measure against is refused rather than compared with a
    /// number this tool invented — the ceiling is the artifact's, and the observation has
    /// to be the host's.
    pub fn measured_slippage_bps(&self, floors: &ArtifactFloors) -> Result<Option<u32>, SimulationError> {
        match (self.slippage_bps, floors.slippage_ceiling_bps) {
            (Some(stated), _) => Ok(Some(stated)),
            (None, Some(ceiling_bps)) => Err(SimulationError::SlippageUnstated { ceiling_bps }),
            (None, None) => Ok(None),
        }
    }

    /// `(net, net in basis points)`, with every refusal that does not need the artifact.
    fn accounting(&self) -> Result<(u128, u32), SimulationError> {
        if self.route.chains.is_empty() {
            return Err(SimulationError::RouteEmpty);
        }
        for (field, amount) in [("gross", &self.gross), ("fees", &self.fees)] {
            if amount.asset != self.capital.asset {
                return Err(SimulationError::AssetMismatch {
                    field,
                    asset: amount.asset.clone(),
                    expected: self.capital.asset.clone(),
                });
            }
        }

        // Every subtraction is checked: a snapshot whose components do not reconcile is
        // refused with its own figures rather than netting to a wrapped number.
        let committed = self
            .capital
            .amount
            .checked_add(self.fees.amount)
            .ok_or(SimulationError::Underwater {
                capital: self.capital.amount,
                fees: self.fees.amount,
                gross: self.gross.amount,
            })?;
        let net = self
            .gross
            .amount
            .checked_sub(committed)
            .ok_or(SimulationError::Underwater {
                capital: self.capital.amount,
                fees: self.fees.amount,
                gross: self.gross.amount,
            })?;

        // The profit as a share of the capital, through PHASE 43's vocabulary rather
        // than a bare ten-thousand division.
        //
        // Zero capital is checked first and by name: `Ratio::of` refuses it too, but it
        // refuses it with the same `None` it uses for a value that does not fit, and
        // "capital is too large to represent" is the wrong thing to tell someone who
        // wrote `0`.
        if self.capital.amount == 0 {
            return Err(SimulationError::ZeroCapital);
        }
        let net_bps = Ratio::of(self.capital.amount, net)
            .ok_or(SimulationError::NotRepresentable {
                capital: self.capital.amount,
                net,
            })?
            .to_bps(RoundingMode::Down)
            .ok_or(SimulationError::NotRepresentable {
                capital: self.capital.amount,
                net,
            })?
            .raw();

        Ok((net, net_bps))
    }
}

/// `1234567` → `1,234,567`.
///
/// Grouping is a presentation choice and this is the only place in the tool that makes
/// it, so that the report reads as the spec's example does without a second number
/// format spreading. It is a pure function of its input.
fn grouped(value: u128) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        // A group boundary sits where the count of digits still to come is a multiple of
        // three, counting from the left. (Counting from the right with `len % 3` is the
        // tempting version and it underflows for a length below the remainder.)
        if index != 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The report, in the shape the phase's example shows.
///
/// A pure function of the outcome, so two simulations of the same snapshot render
/// byte-identical reports (PHASE 42).
pub fn render(outcome: &EconomicOutcome) -> String {
    let asset = &outcome.asset;
    let mut out = String::new();
    out.push_str("Route selected:\n");
    out.push_str(&outcome.chains.join(" → "));
    out.push_str("\n\n");
    if outcome.venues_checked {
        out.push_str(&format!(
            "Venues (checked against the artifact):\n{}\n\n",
            outcome.venues.join(", ")
        ));
    } else {
        out.push_str(&format!(
            "Venues (NOT checked — the artifact declares no approved venue list):\n{}\n\n",
            if outcome.venues.is_empty() {
                "none stated".to_string()
            } else {
                outcome.venues.join(", ")
            }
        ));
    }
    out.push_str(&format!("Capital:\n{} {asset}\n\n", grouped(outcome.capital)));
    out.push_str(&format!("Gross:\n{} {asset}\n\n", grouped(outcome.gross)));
    out.push_str(&format!("Fees:\n{} {asset}\n\n", grouped(outcome.fees)));
    out.push_str(&format!("Net:\n{} {asset}\n\n", grouped(outcome.net)));
    match (outcome.floor_bps, outcome.minimum_required) {
        (Some(floor_bps), Some(minimum)) => {
            out.push_str(&format!(
                "Minimum required:\n{} {asset} ({floor_bps}bps of capital)\n\n",
                grouped(minimum)
            ));
        }
        _ => out.push_str("Minimum required:\nnone — the artifact states no profit floor\n\n"),
    }
    if let (Some(slippage_bps), Some(ceiling_bps)) = (outcome.slippage_bps, outcome.slippage_ceiling_bps) {
        out.push_str(&format!(
            "Slippage:\n{slippage_bps}bps against a ceiling of {ceiling_bps}bps — {}\n\n",
            if slippage_bps <= ceiling_bps { "within" } else { "OVER" }
        ));
    }
    out.push_str(&format!(
        "Result:\n{} (net {}bps{})\n",
        outcome.verdict.label(),
        outcome.net_bps,
        match &outcome.verdict {
            Verdict::BelowFloor { required_bps, .. } => format!(" against a required {required_bps}bps"),
            _ => String::new(),
        }
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The phase's own worked example, in its own units.
    ///
    /// ```text
    /// Capital: 2,000,000 USDC   Gross: 2,019,842 USDC   Fees: 8,119 USDC
    /// Net: 11,723 USDC          Minimum required: 10,000 USDC   Result: PASS
    /// ```
    ///
    /// 2,019,842 − 2,000,000 − 8,119 = 11,723, and 10,000 is 50bps of 2,000,000, so the
    /// example implies a floor of 50bps and a realised 58bps.
    fn spec_example() -> SimulationSnapshot {
        SimulationSnapshot::from_json(
            r#"{
                "version": 1,
                "route": {
                    "chains": ["Ethereum", "Base", "X3"],
                    "venues": ["uniswap", "aerodrome"]
                },
                "capital": { "asset": "USDC", "amount": 2000000 },
                "gross":   { "asset": "USDC", "amount": 2019842 },
                "fees":    { "asset": "USDC", "amount": 8119 },
                "slippage_bps": 12
            }"#,
        )
        .expect("the phase's example is a schema-1 snapshot")
    }

    fn floors_with(profit_floor_bps: Option<u32>, approved: &[&str]) -> ArtifactFloors {
        ArtifactFloors {
            profit_floor_bps,
            slippage_ceiling_bps: None,
            approved_venues: approved.iter().map(|venue| (*venue).to_string()).collect(),
        }
    }

    #[test]
    fn the_phases_own_example_reports_its_own_figures() {
        let outcome = spec_example()
            .evaluate(&floors_with(Some(50), &["uniswap", "aerodrome"]))
            .expect("the example is consistent");

        assert_eq!(outcome.net, 11_723);
        assert_eq!(outcome.net_bps, 58);
        assert_eq!(outcome.floor_bps, Some(50));
        assert_eq!(outcome.minimum_required, Some(10_000));
        assert_eq!(outcome.verdict, Verdict::Pass);
        assert_eq!(outcome.asset, "USDC");
        assert!(outcome.venues_checked);

        let report = render(&outcome);
        for expected in [
            "Route selected:\nEthereum → Base → X3",
            "Capital:\n2,000,000 USDC",
            "Gross:\n2,019,842 USDC",
            "Fees:\n8,119 USDC",
            "Net:\n11,723 USDC",
            "Minimum required:\n10,000 USDC (50bps of capital)",
            "Result:\nPASS (net 58bps)",
        ] {
            assert!(
                report.contains(expected),
                "the report should state {expected:?}; it was:\n{report}"
            );
        }
    }

    #[test]
    fn net_is_derived_so_a_run_below_the_floor_is_a_fail_with_both_figures() {
        // 1,000,000 committed at 2,000,000 gross is 10,000 net — 50bps, against a 60bps
        // floor. The report has to show the floor *and* what was realised, or the reader
        // cannot tell a near miss from a wide one.
        let snapshot = SimulationSnapshot::from_json(
            r#"{
                "version": 1,
                "route": { "chains": ["Ethereum", "X3"], "venues": [] },
                "capital": { "asset": "USDC", "amount": 2000000 },
                "gross":   { "asset": "USDC", "amount": 2010000 },
                "fees":    { "asset": "USDC", "amount": 0 }
            }"#,
        )
        .unwrap();

        let outcome = snapshot.evaluate(&floors_with(Some(60), &[])).unwrap();
        assert_eq!(outcome.net, 10_000);
        assert_eq!(outcome.net_bps, 50);
        assert_eq!(
            outcome.verdict,
            Verdict::BelowFloor {
                net_bps: 50,
                required_bps: 60
            }
        );
        assert_eq!(outcome.verdict.label(), "FAIL");
        assert!(render(&outcome).contains("Result:\nFAIL (net 50bps against a required 60bps)"));
    }

    #[test]
    fn an_artifact_with_no_floor_is_reported_as_no_floor_rather_than_as_a_pass() {
        // A strategy with no profit floor can lose money and still settle. Calling that
        // "PASS" would be the report congratulating a program for stating nothing.
        let outcome = spec_example().evaluate(&floors_with(None, &[])).unwrap();
        assert_eq!(outcome.verdict, Verdict::NoFloorStated);
        assert_eq!(outcome.verdict.label(), "NO FLOOR STATED");
        assert_eq!(outcome.minimum_required, None);
        let report = render(&outcome);
        assert!(report.contains("Minimum required:\nnone — the artifact states no profit floor"));
        assert!(report.contains("Result:\nNO FLOOR STATED"));
    }

    #[test]
    fn a_run_that_returned_less_than_it_committed_is_refused_with_its_figures() {
        let snapshot = SimulationSnapshot::from_json(
            r#"{
                "version": 1,
                "route": { "chains": ["Ethereum", "X3"], "venues": [] },
                "capital": { "asset": "USDC", "amount": 2000000 },
                "gross":   { "asset": "USDC", "amount": 1900000 },
                "fees":    { "asset": "USDC", "amount": 8119 }
            }"#,
        )
        .unwrap();

        let error = snapshot.evaluate(&floors_with(Some(50), &[])).unwrap_err();
        assert_eq!(
            error,
            SimulationError::Underwater {
                capital: 2_000_000,
                fees: 8_119,
                gross: 1_900_000
            }
        );
        // The refusal names all three figures: a shortfall a reader cannot see is a
        // shortfall they cannot act on.
        let message = error.to_string();
        for figure in ["1900000", "2000000", "8119"] {
            assert!(message.contains(figure), "{message} should name {figure}");
        }
    }

    #[test]
    fn zero_capital_is_refused_by_name() {
        // The trap this test exists for: `Ratio::of` refuses zero capital with the same
        // `None` it uses for a value that does not fit, so without the explicit check a
        // snapshot with `"amount": 0` was reported as "too large to represent".
        let snapshot = SimulationSnapshot::from_json(
            r#"{
                "version": 1,
                "route": { "chains": ["X3"], "venues": [] },
                "capital": { "asset": "USDC", "amount": 0 },
                "gross":   { "asset": "USDC", "amount": 100 },
                "fees":    { "asset": "USDC", "amount": 0 }
            }"#,
        )
        .unwrap();

        assert_eq!(
            snapshot.evaluate(&floors_with(Some(50), &[])).unwrap_err(),
            SimulationError::ZeroCapital
        );
    }

    #[test]
    fn amounts_in_different_assets_are_refused_rather_than_netted() {
        // Net is gross less capital less fees. Subtracting a figure in one asset from a
        // figure in another is a claim about a price this tool does not have, so each of
        // the two components is checked against the capital's asset by name.
        let gross_in_eth = r#"{
            "version": 1,
            "route": { "chains": ["X3"], "venues": [] },
            "capital": { "asset": "USDC", "amount": 2000000 },
            "gross":   { "asset": "ETH",  "amount": 2019842 },
            "fees":    { "asset": "USDC", "amount": 8119 }
        }"#;
        let fees_in_eth = r#"{
            "version": 1,
            "route": { "chains": ["X3"], "venues": [] },
            "capital": { "asset": "USDC", "amount": 2000000 },
            "gross":   { "asset": "USDC", "amount": 2019842 },
            "fees":    { "asset": "ETH",  "amount": 8119 }
        }"#;

        for (text, expected_field) in [(gross_in_eth, "gross"), (fees_in_eth, "fees")] {
            let snapshot = SimulationSnapshot::from_json(text).unwrap();
            assert_eq!(
                snapshot.evaluate(&floors_with(Some(50), &[])).unwrap_err(),
                SimulationError::AssetMismatch {
                    field: expected_field,
                    asset: "ETH".to_string(),
                    expected: "USDC".to_string(),
                }
            );
        }
    }

    #[test]
    fn a_snapshot_from_another_schema_version_is_refused() {
        let error = SimulationSnapshot::from_json(
            r#"{
                "version": 2,
                "route": { "chains": ["X3"], "venues": [] },
                "capital": { "asset": "USDC", "amount": 1 },
                "gross":   { "asset": "USDC", "amount": 2 },
                "fees":    { "asset": "USDC", "amount": 0 }
            }"#,
        )
        .unwrap_err();
        assert!(error.contains("schema version 2"), "{error}");
        assert!(error.contains("reads version 1"), "{error}");
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        // A misspelled field is a state the host meant to state and this build cannot
        // see. Ignoring it would simulate a different market and report confidently.
        let error = SimulationSnapshot::from_json(
            r#"{
                "version": 1,
                "route": { "chains": ["X3"], "venues": [] },
                "capital": { "asset": "USDC", "amount": 1 },
                "gross":   { "asset": "USDC", "amount": 2 },
                "fees":    { "asset": "USDC", "amount": 0 },
                "slippage": 12
            }"#,
        )
        .unwrap_err();
        assert!(error.contains("slippage"), "{error}");
    }

    #[test]
    fn a_slippage_ceiling_with_no_stated_slippage_is_refused() {
        let floors = ArtifactFloors {
            profit_floor_bps: Some(50),
            slippage_ceiling_bps: Some(25),
            approved_venues: Vec::new(),
        };
        let snapshot = SimulationSnapshot::from_json(
            r#"{
                "version": 1,
                "route": { "chains": ["X3"], "venues": [] },
                "capital": { "asset": "USDC", "amount": 2000000 },
                "gross":   { "asset": "USDC", "amount": 2019842 },
                "fees":    { "asset": "USDC", "amount": 8119 }
            }"#,
        )
        .unwrap();

        // The accounting itself is fine — it is the ceiling that has nothing to be
        // compared against that is the problem.
        assert!(snapshot.measured_profit_bps().is_ok());
        assert_eq!(
            snapshot.evaluate(&floors).unwrap_err(),
            SimulationError::SlippageUnstated { ceiling_bps: 25 }
        );
    }

    #[test]
    fn a_venue_outside_the_artifacts_approval_is_refused_with_the_list() {
        let error = spec_example()
            .evaluate(&floors_with(Some(50), &["uniswap"]))
            .unwrap_err();
        match error {
            SimulationError::UnapprovedVenue { venue, approved } => {
                assert_eq!(venue, "aerodrome");
                assert_eq!(approved, vec!["uniswap".to_string()]);
            }
            other => panic!("an unapproved venue should be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_tool_that_cannot_check_the_venues_says_so_rather_than_implying_it_did() {
        let outcome = spec_example().evaluate(&floors_with(Some(50), &[])).unwrap();
        assert!(!outcome.venues_checked);
        let report = render(&outcome);
        assert!(
            report.contains("Venues (NOT checked — the artifact declares no approved venue list):"),
            "{report}"
        );
    }

    #[test]
    fn the_same_snapshot_renders_a_byte_identical_report() {
        // PHASE 42: nothing here reads a clock, a random source or a hash map, so two
        // simulations of one snapshot and one artifact are the same bytes.
        let floors = floors_with(Some(50), &["uniswap", "aerodrome"]);
        let first = render(&spec_example().evaluate(&floors).unwrap());
        for _ in 0..8 {
            assert_eq!(render(&spec_example().evaluate(&floors).unwrap()), first);
        }
    }

    #[test]
    fn an_empty_route_is_refused() {
        let snapshot = SimulationSnapshot::from_json(
            r#"{
                "version": 1,
                "route": { "chains": [], "venues": [] },
                "capital": { "asset": "USDC", "amount": 1 },
                "gross":   { "asset": "USDC", "amount": 2 },
                "fees":    { "asset": "USDC", "amount": 0 }
            }"#,
        )
        .unwrap();
        assert_eq!(snapshot.measured_profit_bps().unwrap_err(), SimulationError::RouteEmpty);
    }

    #[test]
    fn figures_are_grouped_in_threes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(11_723), "11,723");
        assert_eq!(grouped(2_019_842), "2,019,842");
        assert_eq!(grouped(1_234_567_890), "1,234,567,890");
    }
}
