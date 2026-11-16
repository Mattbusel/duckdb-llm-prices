use duckdb_neo::{Result, duckdb_entrypoint_c_api_v2, extension::Extension};

#[duckdb_entrypoint_c_api_v2(extension_name = "llm_prices", api_version = "2.0.0")]
pub fn entrypoint(extension: &Extension) -> Result<()> {
    llm_prices_duckdb::register(extension)
}
