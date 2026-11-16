//! `llm_prices`: LLM API prices and token costs in DuckDB SQL, offline.
//!
//! The price table is the [`llm-prices`](https://crates.io/crates/llm-prices) crate, compiled
//! into the extension: one dated, source-cited row per model for Anthropic, OpenAI, Google,
//! DeepSeek, Mistral, xAI, Cohere, Groq, Together AI and Fireworks AI. Nothing is fetched at
//! query time and no key is needed.
//!
//! | Function | Returns |
//! |---|---|
//! | `llm_cost(model, input_tokens, output_tokens [, cached_tokens])` | USD cost of one request (`DOUBLE`), `NULL` for an unknown model |
//! | `llm_model_id(model)` | the table's model id for a provider-prefixed, dated or aliased id |
//! | `llm_prices()` | one row per model with every price, USD per 1M tokens |
//! | `llm_prices_version()` | the date the bundled table was checked against the providers' pages |

use duckdb_neo::{Result, extension::Extension};

mod cost;
mod table;

/// The category every function of this extension is documented under in `duckdb_functions()`.
pub const CATEGORY: &str = "llm_prices";

/// Register everything the extension provides.
pub fn register(extension: &Extension) -> Result<()> {
    cost::register(extension)?;
    table::register(extension)?;
    Ok(())
}
