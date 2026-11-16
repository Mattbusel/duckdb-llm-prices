//! The scalar functions: `llm_cost`, `llm_model_id` and `llm_prices_version`.

use std::collections::HashMap;

use duckdb_neo::{
    DuckDBType, Result, ToValue,
    connection::Context,
    data_chunk::VectorCollection,
    error::Error,
    extension::Extension,
    scalar::{ScalarCallbacks, ScalarFunctionBuilder},
    signature::{
        FunctionDocumentationBuilder, FunctionParameter, FunctionParameterKind, SignatureBuilder,
    },
    vector::{StorageKind, Unknown, Vector},
};
use prices::{Price, Provider, Usage};

use crate::CATEGORY;

/// Flatten argument vectors DuckDB hands over in a layout that cannot be read row by row
/// (sequence, FSST).
fn flatten_compressed(vectors: &mut VectorCollection) -> Result<()> {
    for index in 0..vectors.col_count() {
        let mut vector = vectors.get_vector_at_mut::<Unknown>(index)?;
        if vector.storage_kind() == StorageKind::Other {
            vector.flatten()?;
        }
    }
    Ok(())
}

/// Resolves `(provider, model)` to a row once per distinct pair in a chunk.
#[derive(Default)]
struct Resolver {
    cache: HashMap<(String, String), Option<&'static Price>>,
}

impl Resolver {
    fn get(
        &mut self,
        function: &str,
        provider: &str,
        model: &str,
    ) -> Result<Option<&'static Price>> {
        let key = (provider.to_string(), model.to_string());
        if let Some(found) = self.cache.get(&key) {
            return Ok(*found);
        }
        let found = if provider.trim().is_empty() {
            prices::lookup(model)
        } else {
            let parsed = Provider::parse(provider).ok_or_else(|| {
                let known: Vec<&str> = Provider::ALL.iter().map(|p| p.as_str()).collect();
                Error::invalid_input(format!(
                    "{function}: unknown provider '{provider}'; use one of {}, or '' to search every provider",
                    known.join(", ")
                ))
            })?;
            parsed.lookup(model)
        };
        self.cache.insert(key, found);
        Ok(found)
    }
}

fn tokens(function: &str, what: &str, value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| {
        Error::invalid_input(format!("{function}: {what} must be 0 or more, got {value}"))
    })
}

/// `llm_cost(model, input_tokens, output_tokens, cached_tokens := 0,
/// cache_write_5m_tokens := 0, cache_write_1h_tokens := 0, batch := false, provider := '')`.
struct LlmCost;

impl ScalarCallbacks for LlmCost {
    type BindData = ();
    type InitData = ();

    fn exec(
        &self,
        _context: &Context,
        _bind_data: Option<&Self::BindData>,
        _init_data: Option<&mut Self::InitData>,
        vectors: &mut VectorCollection,
        output: Vector<'_, Unknown>,
    ) -> Result<()> {
        const NAME: &str = "llm_cost";
        let rows = vectors.row_count()?;
        flatten_compressed(vectors)?;
        let mut output = output.cast::<f64>()?;
        output.set_size(rows)?;

        let model = vectors.get_vector_at::<String>(0)?.reader()?;
        let input = vectors.get_vector_at::<i64>(1)?.reader()?;
        let out = vectors.get_vector_at::<i64>(2)?.reader()?;
        let cached = vectors.get_vector_at::<i64>(3)?.reader()?;
        let write_5m = vectors.get_vector_at::<i64>(4)?.reader()?;
        let write_1h = vectors.get_vector_at::<i64>(5)?.reader()?;
        let batch = vectors.get_vector_at::<bool>(6)?.reader()?;
        let provider = vectors.get_vector_at::<String>(7)?.reader()?;

        let mut resolver = Resolver::default();
        let mut writer = output.writer()?;
        for row in 0..rows {
            let (
                Some(model),
                Some(input),
                Some(out),
                Some(cached),
                Some(write_5m),
                Some(write_1h),
                Some(batch),
                Some(provider),
            ) = (
                model.get(row)?,
                input.get(row)?,
                out.get(row)?,
                cached.get(row)?,
                write_5m.get(row)?,
                write_1h.get(row)?,
                batch.get(row)?,
                provider.get(row)?,
            )
            else {
                writer.set_null(row)?;
                continue;
            };
            let usage = Usage {
                input: tokens(NAME, "input_tokens", *input)?,
                output: tokens(NAME, "output_tokens", *out)?,
                cache_read: tokens(NAME, "cached_tokens", *cached)?,
                cache_write_5m: tokens(NAME, "cache_write_5m_tokens", *write_5m)?,
                cache_write_1h: tokens(NAME, "cache_write_1h_tokens", *write_1h)?,
                batch: *batch,
            };
            let cost = resolver
                .get(NAME, provider, model)?
                .and_then(|price| price.cost_usage(&usage))
                .map(|c| c.total);
            match cost {
                Some(cost) => writer.set(row, &cost)?,
                None => writer.set_null(row)?,
            }
        }
        Ok(())
    }
}

/// `llm_model_id(model, provider := '')`: the id of the row a model id resolves to.
struct LlmModelId;

impl ScalarCallbacks for LlmModelId {
    type BindData = ();
    type InitData = ();

    fn exec(
        &self,
        _context: &Context,
        _bind_data: Option<&Self::BindData>,
        _init_data: Option<&mut Self::InitData>,
        vectors: &mut VectorCollection,
        output: Vector<'_, Unknown>,
    ) -> Result<()> {
        let rows = vectors.row_count()?;
        flatten_compressed(vectors)?;
        let mut output = output.cast::<String>()?;
        output.set_size(rows)?;

        let model = vectors.get_vector_at::<String>(0)?.reader()?;
        let provider = vectors.get_vector_at::<String>(1)?.reader()?;
        let mut resolver = Resolver::default();
        let mut writer = output.writer()?;
        for row in 0..rows {
            let found = match (model.get(row)?, provider.get(row)?) {
                (Some(model), Some(provider)) => resolver.get("llm_model_id", provider, model)?,
                _ => None,
            };
            match found {
                Some(price) => writer.set(row, price.model)?,
                None => writer.set_null(row)?,
            }
        }
        Ok(())
    }
}

/// `llm_prices_version()`: the date the bundled table was generated.
struct Version;

impl ScalarCallbacks for Version {
    type BindData = ();
    type InitData = ();

    fn exec(
        &self,
        _context: &Context,
        _bind_data: Option<&Self::BindData>,
        _init_data: Option<&mut Self::InitData>,
        vectors: &mut VectorCollection,
        output: Vector<'_, Unknown>,
    ) -> Result<()> {
        let rows = vectors.row_count()?;
        let mut output = output.cast::<String>()?;
        output.set_size(rows)?;
        let mut writer = output.writer()?;
        for row in 0..rows {
            writer.set(row, prices::GENERATED)?;
        }
        Ok(())
    }
}

/// A parameter callers pass by name only, such as `provider := 'groq'`.
fn named<T: DuckDBType + ToValue + 'static>(name: &str, default: T) -> FunctionParameter {
    FunctionParameter::with_default(name, default).kind(FunctionParameterKind::Named)
}

pub(crate) fn register(extension: &Extension) -> Result<()> {
    ScalarFunctionBuilder::new(
        "llm_cost",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("model"))
            .param(FunctionParameter::new::<i64>("input_tokens"))
            .param(FunctionParameter::new::<i64>("output_tokens"))
            .param(FunctionParameter::with_default("cached_tokens", 0_i64))
            .param(named("cache_write_5m_tokens", 0_i64))
            .param(named("cache_write_1h_tokens", 0_i64))
            .param(named("batch", false))
            .param(named("provider", String::new()))
            .return_type(f64::logical_type(extension)?)
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description(
                        "USD cost of one LLM API request from its token counts, priced offline from the bundled llm-prices table. \
                         input_tokens are uncached input tokens (for OpenAI usage: prompt_tokens minus cached_tokens); \
                         cached_tokens are input tokens read from the prompt cache. \
                         Long-context prices apply when the prompt is over the model's threshold. \
                         Accepts provider prefixes (anthropic/, openrouter/), Bedrock and Vertex ids, dated snapshots and aliases. \
                         Returns NULL for a model the table does not price.",
                    )
                    .add_example("One request", "SELECT llm_cost('gpt-4o', 1200, 350)")
                    .add_example(
                        "With cache reads",
                        "SELECT llm_cost('claude-sonnet-4-5', 2000, 400, 18000)",
                    )
                    .add_example(
                        "Spend per model",
                        "SELECT model, sum(llm_cost(model, input_tokens, output_tokens)) AS usd FROM requests GROUP BY model",
                    )
                    .add_example(
                        "One provider",
                        "SELECT llm_cost('openai/gpt-oss-120b', 1000, 1000, provider := 'groq')",
                    )
                    .add_category(CATEGORY),
            ),
        LlmCost,
    )
    .register(extension)?;

    ScalarFunctionBuilder::new(
        "llm_model_id",
        SignatureBuilder::new()
            .param(FunctionParameter::new::<String>("model"))
            .param(named("provider", String::new()))
            .return_type(String::logical_type(extension)?)
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description(
                        "The model id of the llm_prices() row that a provider-prefixed, dated, Bedrock, Vertex or aliased id resolves to, for joining usage logs to llm_prices(). NULL when the table has no such model.",
                    )
                    .add_example(
                        "Normalize an id",
                        "SELECT llm_model_id('us.anthropic.claude-sonnet-4-5-20250929-v1:0')",
                    )
                    .add_category(CATEGORY),
            ),
        LlmModelId,
    )
    .register(extension)?;

    ScalarFunctionBuilder::new(
        "llm_prices_version",
        SignatureBuilder::new()
            .return_type(String::logical_type(extension)?)
            .documentation(
                FunctionDocumentationBuilder::new()
                    .description(
                        "The date (YYYY-MM-DD) the bundled llm-prices table was checked against the providers' pricing pages.",
                    )
                    .add_example("Data date", "SELECT llm_prices_version()")
                    .add_category(CATEGORY),
            ),
        Version,
    )
    .register(extension)
}
