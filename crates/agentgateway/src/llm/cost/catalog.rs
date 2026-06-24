use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Catalog {
	#[serde(default)]
	pub providers: BTreeMap<String, Provider>,
}

impl Catalog {
	pub fn validate(&self) -> anyhow::Result<()> {
		for (pid, p) in &self.providers {
			for (mid, m) in &p.models {
				let mut prev: Option<u64> = None;
				for (i, t) in m.tiers.iter().enumerate() {
					if prev.is_some_and(|p| t.context_over <= p) {
						anyhow::bail!(
							"{pid}/{mid}: tier {i} threshold {} not strictly greater than previous",
							t.context_over
						);
					}
					prev = Some(t.context_over);
				}
			}
		}
		Ok(())
	}

	pub fn override_with(mut self, overlay: Catalog) -> Catalog {
		for (pid, op) in overlay.providers {
			match self.providers.get_mut(&pid) {
				Some(base) => base.models.extend(op.models),
				None => {
					self.providers.insert(pid, op);
				},
			}
		}
		self
	}

	pub fn resolve(&self, provider: &str, model: &str) -> Option<&Model> {
		self.providers.get(provider)?.models.get(model)
	}
}

pub fn from_json(s: &str) -> anyhow::Result<Catalog> {
	let catalog: Catalog = serde_json::from_str(s)?;
	catalog.validate()?;
	Ok(catalog)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Provider {
	#[serde(default)]
	pub models: BTreeMap<String, Model>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Model {
	#[serde(default, skip_serializing_if = "Rates::is_empty")]
	pub rates: Rates,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub tiers: Vec<Tier>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rates {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub input: Option<Money>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub output: Option<Money>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cache_read: Option<Money>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cache_write: Option<Money>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub reasoning: Option<Money>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub input_audio: Option<Money>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub output_audio: Option<Money>,
}

impl Rates {
	pub fn is_empty(&self) -> bool {
		*self == Rates::default()
	}

	pub fn overlay(&self, delta: &Rates) -> Rates {
		let pick = |base: &Option<Money>, d: &Option<Money>| d.clone().or_else(|| base.clone());
		Rates {
			input: pick(&self.input, &delta.input),
			output: pick(&self.output, &delta.output),
			cache_read: pick(&self.cache_read, &delta.cache_read),
			cache_write: pick(&self.cache_write, &delta.cache_write),
			reasoning: pick(&self.reasoning, &delta.reasoning),
			input_audio: pick(&self.input_audio, &delta.input_audio),
			output_audio: pick(&self.output_audio, &delta.output_audio),
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tier {
	pub context_over: u64,
	pub rates: Rates,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Money(pub Decimal);

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Money {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"Money".into()
	}

	fn json_schema(schema_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
		String::json_schema(schema_gen)
	}
}

impl Money {
	pub fn parse(s: &str) -> Result<Money, String> {
		let d = Decimal::from_str(s).map_err(|e| format!("invalid decimal {s:?}: {e}"))?;
		if d < Decimal::ZERO {
			return Err(format!("negative rate not allowed: {s:?}"));
		}
		if d.scale() > 6 {
			return Err(format!("more than 6 fractional digits: {s:?}"));
		}
		Ok(Money(d))
	}
}

impl fmt::Display for Money {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl TryFrom<String> for Money {
	type Error = String;

	fn try_from(s: String) -> Result<Money, String> {
		Money::parse(&s)
	}
}

impl From<Money> for String {
	fn from(m: Money) -> String {
		m.0.to_string()
	}
}
const TOKENS_PER_UNIT: u64 = 1_000_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
	pub input: u64,
	pub cache_read: u64,
	pub cache_write: u64,
	pub output: u64,
	pub reasoning: u64,
	pub input_audio: u64,
	pub output_audio: u64,
}

impl Usage {
	pub fn context_tokens(&self) -> u64 {
		self
			.input
			.saturating_add(self.cache_read)
			.saturating_add(self.cache_write)
			.saturating_add(self.input_audio)
	}
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Breakdown {
	pub input: Decimal,
	pub cache_read: Decimal,
	pub cache_write: Decimal,
	pub output: Decimal,
	pub reasoning: Decimal,
	pub input_audio: Decimal,
	pub output_audio: Decimal,
}

impl Breakdown {
	pub fn total(&self) -> Decimal {
		self.input
			+ self.cache_read
			+ self.cache_write
			+ self.output
			+ self.reasoning
			+ self.input_audio
			+ self.output_audio
	}
}

impl Rates {
	pub fn breakdown(&self, usage: &Usage) -> Breakdown {
		let unit = Decimal::from(TOKENS_PER_UNIT);
		// Audio and reasoning counts are provider-reported subsets of text totals.
		// If no dedicated modality rate exists, bill them at the text rate.
		let reasoning_rate = self.reasoning.as_ref().or(self.output.as_ref());
		let input_audio_rate = self.input_audio.as_ref().or(self.input.as_ref());
		let output_audio_rate = self.output_audio.as_ref().or(self.output.as_ref());
		Breakdown {
			input: line(usage.input, self.input.as_ref()) / unit,
			cache_read: line(usage.cache_read, self.cache_read.as_ref()) / unit,
			cache_write: line(usage.cache_write, self.cache_write.as_ref()) / unit,
			output: line(usage.output, self.output.as_ref()) / unit,
			reasoning: line(usage.reasoning, reasoning_rate) / unit,
			input_audio: line(usage.input_audio, input_audio_rate) / unit,
			output_audio: line(usage.output_audio, output_audio_rate) / unit,
		}
	}
}

impl Model {
	#[cfg(test)]
	pub fn price(&self, usage: &Usage) -> Decimal {
		self.breakdown(usage).total()
	}

	#[cfg(test)]
	pub fn breakdown(&self, usage: &Usage) -> Breakdown {
		self
			.effective_rates(usage.context_tokens())
			.breakdown(usage)
	}

	pub(super) fn effective_rates(&self, context_tokens: u64) -> Rates {
		match self
			.tiers
			.iter()
			.filter(|t| context_tokens > t.context_over)
			.max_by_key(|t| t.context_over)
		{
			Some(tier) => self.rates.overlay(&tier.rates),
			None => self.rates.clone(),
		}
	}
}

fn line(tokens: u64, rate: Option<&Money>) -> Decimal {
	match rate {
		Some(Money(r)) => Decimal::from(tokens) * *r,
		None => Decimal::ZERO,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn m(s: &str) -> Money {
		Money::parse(s).unwrap()
	}

	fn d(s: &str) -> Decimal {
		Decimal::from_str(s).unwrap()
	}

	fn entry(rates: Rates, tiers: Vec<Tier>) -> Model {
		Model { rates, tiers }
	}

	fn tier(context_over: u64, rates: Rates) -> Tier {
		Tier {
			context_over,
			rates,
		}
	}

	fn rates(input: &str, output: &str) -> Rates {
		Rates {
			input: Some(m(input)),
			output: Some(m(output)),
			..Default::default()
		}
	}

	const GOLDEN_CATALOG: &str = include_str!("testdata/model_catalog.golden.json");

	#[test]
	fn contract_parses_and_round_trips() {
		let catalog = from_json(GOLDEN_CATALOG).expect("golden catalog must parse");

		let reemitted = serde_json::to_string(&catalog).unwrap();
		let reparsed = from_json(&reemitted).unwrap();
		assert_eq!(catalog, reparsed);

		let gemini = &catalog.providers["gcp.gemini"].models["gemini-2.5-pro"];
		assert_eq!(gemini.tiers.len(), 1);
		assert_eq!(gemini.tiers[0].context_over, 200_000);
		assert_eq!(
			gemini.rates.cache_read,
			Some(Money::parse("0.125").unwrap())
		);

		let audio = &catalog.providers["openai"].models["gpt-4o-mini-audio-preview"];
		assert_eq!(audio.rates.input_audio, Some(Money::parse("10").unwrap()));
		assert_eq!(audio.rates.output_audio, Some(Money::parse("20").unwrap()));

		let anthropic = &catalog.providers["anthropic"].models["claude-sonnet-4-5"];
		assert_eq!(
			anthropic.rates.cache_write,
			Some(Money::parse("3.75").unwrap())
		);

		let mini = catalog.resolve("openai", "gpt-4o-mini");
		assert!(mini.is_some(), "OpenAI entry resolves");
		assert!(
			!mini.unwrap().effective_rates(0).is_empty(),
			"and is priced"
		);
	}

	#[test]
	fn limits_are_rejected() {
		let err = from_json(
			r#"{"providers":{"openai":{"models":{"m":{"rates":{"input":"1"},"limits":{"contextWindow":128000}}}}}}"#,
		)
		.unwrap_err();
		assert!(err.to_string().contains("unknown field"), "{err}");
	}

	#[test]
	fn resolve_is_provider_scoped() {
		let catalog =
			from_json(r#"{"providers":{"openai":{"models":{"shared":{"rates":{"input":"5"}}}}}}"#)
				.unwrap();
		assert!(catalog.resolve("openai", "shared").is_some());
		assert!(
			catalog.resolve("custom", "shared").is_none(),
			"another provider's model does not leak"
		);
		assert!(catalog.resolve("openai", "no-such-model").is_none());
	}

	#[test]
	fn money_is_a_string_not_a_number() {
		assert_eq!(serde_json::to_string(&m("0.175")).unwrap(), "\"0.175\"");
		let ok: Rates = serde_json::from_str(r#"{"input": "0.175"}"#).unwrap();
		assert_eq!(ok.input, Some(m("0.175")));
		let err = serde_json::from_str::<Rates>(r#"{"input": 0.175}"#).unwrap_err();
		assert!(err.to_string().contains("string"), "{err}");
	}

	#[test]
	fn money_validation() {
		assert!(Money::parse("-1").is_err(), "negative rejected");
		assert!(Money::parse("0.1234567").is_err(), ">6 dp rejected");
		assert!(Money::parse("0").is_ok());
		assert!(Money::parse("0.123456").is_ok());
	}

	#[test]
	fn unknown_field_is_rejected() {
		let err = serde_json::from_str::<Rates>(r#"{"inputCacheRead": "1"}"#).unwrap_err();
		assert!(err.to_string().contains("unknown field"), "{err}");
	}

	#[test]
	fn rates_overlay_prefers_delta() {
		let base = Rates {
			input: Some(m("5")),
			output: Some(m("25")),
			..Default::default()
		};
		let delta = Rates {
			input: Some(m("6")),
			..Default::default()
		};
		let merged = base.overlay(&delta);
		assert_eq!(merged.input, Some(m("6")), "delta wins");
		assert_eq!(
			merged.output,
			Some(m("25")),
			"base kept where delta is absent"
		);
	}

	#[test]
	fn override_replaces_whole_entry_and_keeps_siblings() {
		let base: Catalog = serde_json::from_str(
			r#"{"providers":{"openai":{"models":{
					"gpt-5":{"rates":{"input":"1.25","output":"10"}},
					"gpt-4o":{"rates":{"input":"2.5","output":"10"}}
				}}}}"#,
		)
		.unwrap();
		let overlay: Catalog = serde_json::from_str(
			r#"{"providers":{"openai":{"models":{"gpt-5":{"rates":{"output":"8"}}}}}}"#,
		)
		.unwrap();
		let merged = base.override_with(overlay);
		let gpt5 = &merged.providers["openai"].models["gpt-5"];
		assert_eq!(gpt5.rates.output, Some(m("8")));
		assert_eq!(
			gpt5.rates.input, None,
			"whole-entry replace drops base fields"
		);
		assert_eq!(
			merged.providers["openai"].models["gpt-4o"].rates.input,
			Some(m("2.5")),
			"sibling untouched"
		);
	}

	#[test]
	fn breakdown_components_sum_to_price() {
		let e = entry(
			Rates {
				input: Some(m("3")),
				output: Some(m("15")),
				cache_read: Some(m("0.3")),
				cache_write: Some(m("3.75")),
				reasoning: Some(m("15")),
				input_audio: Some(m("40")),
				output_audio: Some(m("80")),
			},
			vec![],
		);
		let u = Usage {
			input: 1000,
			cache_read: 2000,
			cache_write: 500,
			output: 500,
			reasoning: 100,
			input_audio: 50,
			output_audio: 25,
		};
		let b = e.breakdown(&u);
		assert_eq!(b.input, d("0.003"));
		assert_eq!(b.cache_read, d("0.0006"));
		assert_eq!(b.cache_write, d("0.001875"));
		assert_eq!(b.output, d("0.0075"));
		assert_eq!(b.reasoning, d("0.0015"));
		assert_eq!(b.input_audio, d("0.002"));
		assert_eq!(b.output_audio, d("0.002"));
		assert_eq!(b.total(), e.price(&u));
		assert_eq!(b.total(), d("0.018475"));
	}

	#[test]
	fn absent_modality_rate_falls_back_to_text() {
		let e = entry(rates("3", "15"), vec![]);
		let u = Usage {
			input: 1000,
			output: 500,
			reasoning: 2000,
			input_audio: 100,
			output_audio: 50,
			..Default::default()
		};
		let b = e.breakdown(&u);
		assert_eq!(
			b.reasoning,
			d("0.03"),
			"reasoning falls back to output rate"
		);
		assert_eq!(
			b.input_audio,
			d("0.0003"),
			"input audio falls back to input rate"
		);
		assert_eq!(
			b.output_audio,
			d("0.00075"),
			"output audio falls back to output rate"
		);
	}

	#[test]
	fn absent_cache_rate_is_not_charged() {
		let e = entry(rates("3", "15"), vec![]);
		let u = Usage {
			input: 1000,
			cache_read: 5000,
			..Default::default()
		};
		let b = e.breakdown(&u);
		assert_eq!(b.cache_read, Decimal::ZERO, "no cache rate -> not billed");
	}

	#[test]
	fn tier_reprices_the_whole_request() {
		let e = entry(rates("1.25", "10"), vec![tier(200_000, rates("2.5", "15"))]);
		let usage = |input| Usage {
			input,
			output: 1000,
			..Default::default()
		};
		assert_eq!(e.price(&usage(100_000)), d("0.135"), "below threshold");
		assert_eq!(
			e.price(&usage(200_000)),
			d("0.26"),
			"exact threshold stays base"
		);
		assert_eq!(
			e.price(&usage(250_000)),
			d("0.64"),
			"above threshold whole-request reprice"
		);
	}

	#[test]
	fn highest_applicable_tier_wins() {
		let e = entry(
			Rates {
				input: Some(m("1")),
				..Default::default()
			},
			vec![
				tier(
					100_000,
					Rates {
						input: Some(m("2")),
						..Default::default()
					},
				),
				tier(
					500_000,
					Rates {
						input: Some(m("4")),
						..Default::default()
					},
				),
			],
		);
		let u = Usage {
			input: 600_000,
			..Default::default()
		};
		assert_eq!(e.price(&u), d("2.4"));
	}

	#[test]
	fn tier_omitted_component_falls_back_to_base() {
		let e = entry(
			rates("1.25", "10"),
			vec![tier(
				200_000,
				Rates {
					input: Some(m("2.5")),
					..Default::default()
				},
			)],
		);
		let u = Usage {
			input: 250_000,
			output: 1000,
			..Default::default()
		};
		let b = e.breakdown(&u);
		assert_eq!(b.input, d("0.625"));
		assert_eq!(
			b.output,
			d("0.01"),
			"omitted output falls back to base rate"
		);
	}

	#[test]
	fn has_pricing_depends_on_effective_tier() {
		let input_rate = Rates {
			input: Some(m("1")),
			..Default::default()
		};
		assert!(
			entry(Rates::default(), vec![])
				.effective_rates(0)
				.is_empty()
		);
		assert!(
			!entry(input_rate.clone(), vec![])
				.effective_rates(0)
				.is_empty()
		);

		let tier_only = entry(Rates::default(), vec![tier(100_000, input_rate)]);
		assert!(tier_only.effective_rates(100_000).is_empty());
		assert!(!tier_only.effective_rates(100_001).is_empty());

		assert!(
			entry(Rates::default(), vec![tier(100_000, Rates::default())])
				.effective_rates(100_001)
				.is_empty()
		);
	}

	#[test]
	fn sub_micro_amounts_are_exact() {
		let e = entry(
			Rates {
				input: Some(m("0.075")),
				..Default::default()
			},
			vec![],
		);
		let u = Usage {
			input: 333,
			..Default::default()
		};
		assert_eq!(e.price(&u), d("0.000024975"));
	}
}
