# llm_prices for DuckDB

Price LLM API usage in SQL. The extension bundles the [llm-prices](https://crates.io/crates/llm-prices) table (146 models from Anthropic, OpenAI, Google, DeepSeek, Mistral, xAI, Cohere, Groq, Together AI and Fireworks AI, each row dated and linked to the provider's pricing page) and prices token counts offline. No network calls and no API key.

```sql
INSTALL llm_prices FROM community;
LOAD llm_prices;

-- One request: 1,200 input and 350 output tokens on gpt-4o
SELECT llm_cost('gpt-4o', 1200, 350);          -- 0.0065

-- Spend per model from a usage log (Langfuse / Helicone export, gateway logs, OTel gen_ai spans)
SELECT llm_model_id(model) AS model,
       count(*)            AS requests,
       sum(llm_cost(model, input_tokens, output_tokens, cached_tokens)) AS usd
FROM 'usage/*.parquet'
GROUP BY ALL
ORDER BY usd DESC;

-- The price table itself
SELECT provider, model, input, output, cache_read, checked
FROM llm_prices()
WHERE status = 'listed'
ORDER BY output;
```

## Functions

| Function | Returns |
| --- | --- |
| `llm_cost(model, input_tokens, output_tokens, cached_tokens := 0)` | USD cost of one request as `DOUBLE`; `NULL` when the table does not price the model |
| `llm_model_id(model, provider := '')` | the `llm_prices()` model id a raw id resolves to, for joins |
| `llm_prices()` | one row per model with every price in USD per 1M tokens |
| `llm_prices_version()` | the date (`YYYY-MM-DD`) the bundled table was checked against the providers' pages |

### llm_cost

```sql
llm_cost(model VARCHAR, input_tokens BIGINT, output_tokens BIGINT,
         cached_tokens BIGINT := 0,
         cache_write_5m_tokens := 0, cache_write_1h_tokens := 0,
         batch := false, provider := '') -> DOUBLE
```

- `input_tokens` are uncached input tokens. OpenAI's `usage.prompt_tokens` includes cached tokens, so pass `prompt_tokens - prompt_tokens_details.cached_tokens`. Anthropic's `input_tokens` is already uncached.
- `cached_tokens` are input tokens read from the prompt cache (Anthropic `cache_read_input_tokens`, OpenAI `prompt_tokens_details.cached_tokens`, Gemini `cachedContentTokenCount`).
- `cache_write_5m_tokens` and `cache_write_1h_tokens` are Anthropic cache writes (`cache_creation.ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens`). A row that does not price that cache lifetime returns `NULL`.
- `batch := true` uses Batch API prices.
- `provider := 'groq'` picks one provider's row when several sell the same model id (`openai/gpt-oss-120b` on Groq, Together and Fireworks). Provider names follow LiteLLM and LangChain spellings too (`google_genai`, `together_ai`, `x-ai`). Without it, the first matching row in table order wins.
- Long-context prices apply automatically when the prompt (input plus cache reads plus cache writes) is over the model's threshold.
- Model ids are matched the way the crate's `lookup` does: any ASCII case, provider prefixes (`anthropic/`, `openrouter/`, `models/`), Bedrock ids (`us.anthropic.claude-...-v1:0`), Vertex `@date` suffixes, dated snapshots and `-latest` aliases.
- A `NULL` argument gives `NULL`. A negative token count or an unknown provider name is an error.

Token columns must be integers up to `BIGINT`. Cast a `HUGEINT` (the type of `sum(BIGINT)`) with `::BIGINT`, or better, price each request and sum the costs.

### llm_prices()

| Column | Type | Meaning |
| --- | --- | --- |
| `provider` | VARCHAR | `anthropic`, `openai`, `google`, `deepseek`, `mistral`, `xai`, `cohere`, `groq`, `together`, `fireworks` |
| `model` | VARCHAR | the API model id |
| `aliases` | VARCHAR[] | other ids the provider documents for the same model |
| `status` | VARCHAR | `listed`, `retired` (still sold on some clouds) or `limited` |
| `input`, `output`, `cache_read` | DOUBLE | USD per 1M tokens |
| `cache_write_5m`, `cache_write_1h`, `cache_write_30m`, `cache_write_12h` | DOUBLE | cache write price per lifetime |
| `batch_input`, `batch_output` | DOUBLE | Batch API prices |
| `long_context_threshold` | BIGINT | prompts over this many tokens use the long-context prices |
| `long_context_input`, `long_context_output`, `long_context_cache_read` | DOUBLE | long-context prices |
| `source_url` | VARCHAR | the pricing page the row was read from |
| `checked` | DATE | when the row was checked against `source_url` |

A missing price is `NULL`.

## Keeping prices current

The table is compiled in, so a query never changes cost because a web page changed. Each llm-prices release is followed by an extension release that bumps the bundled table; `llm_prices_version()` tells you which one you have. Report a wrong or missing price at https://github.com/Mattbusel/llm-prices/issues with the provider's pricing page.

## Building

```shell
git clone --recurse-submodules https://github.com/Mattbusel/duckdb-llm-prices
cd duckdb-llm-prices
make configure
make release
make test_release
```

This is the [DuckDB Rust extension template](https://github.com/duckdb/extension-template-rs) (C API v2, `duckdb-neo`). The binaries load in DuckDB v2.0.0.
