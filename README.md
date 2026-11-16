# llm_prices for DuckDB

A DuckDB extension that prices LLM API usage in SQL. It bundles the [llm-prices](https://crates.io/crates/llm-prices) table (146 models from Anthropic, OpenAI, Google, DeepSeek, Mistral, xAI, Cohere, Groq, Together AI and Fireworks AI, each row dated and linked to the provider's pricing page) and prices token counts offline. No network calls and no API key.

## Install

The extension is submitted to DuckDB community extensions ([duckdb/community-extensions#3639](https://github.com/duckdb/community-extensions/pull/3639)). Once it is listed:

```sql
INSTALL llm_prices FROM community;
LOAD llm_prices;
```

Until then, build it (needs Rust, Python 3 and make) and load the file into DuckDB v2.0.0 started with `duckdb -unsigned`:

```sh
git clone --recurse-submodules https://github.com/Mattbusel/duckdb-llm-prices
cd duckdb-llm-prices
make configure && make release
duckdb -unsigned -c "LOAD 'build/release/llm_prices.duckdb_extension'; SELECT llm_cost('gpt-4o', 1200, 350);"
```

## Example

```sql
-- One request: 1,200 input and 350 output tokens on gpt-4o
SELECT llm_cost('gpt-4o', 1200, 350);          -- 0.0065

-- Spend per model from a usage log
SELECT llm_model_id(model) AS model,
       count(*)            AS requests,
       sum(llm_cost(model, input_tokens, output_tokens, cached_tokens)) AS usd
FROM 'usage/*.parquet'
GROUP BY ALL
ORDER BY usd DESC;
```

`llm_cost` returns `NULL` when the table does not price the model. `llm_prices()` returns the table itself.

## Docs and license

- Functions, arguments and columns: [docs/USAGE.md](https://github.com/Mattbusel/duckdb-llm-prices/blob/main/docs/USAGE.md)
- MIT license: [LICENSE](https://github.com/Mattbusel/duckdb-llm-prices/blob/main/LICENSE)
