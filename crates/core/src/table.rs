//! The `llm_prices()` table function: one row per model in the bundled table.

use std::sync::atomic::{AtomicUsize, Ordering};

use duckdb_neo::{
    DuckDBType, Result,
    bind_arguments::BindArgument,
    connection::Context,
    data_chunk::DataChunkRef,
    error::Error,
    extension::Extension,
    logical_type::LogicalType,
    signature::{FunctionDocumentationBuilder, SignatureBuilder},
    table_function::{
        BindFunctionHandle, ExecColumnInfo, InitColumnData, TableFunctionBuilder,
        TableFunctionCallbacks,
    },
    types::{DateValue, List},
};
use prices::{Price, Rate};

use crate::CATEGORY;

/// What a column holds, so `exec` knows which vector type to write.
#[derive(Clone, Copy)]
enum Kind {
    Text(fn(&Price) -> Option<&'static str>),
    Usd(fn(&Price) -> Option<f64>),
    Tokens(fn(&Price) -> Option<i64>),
    Aliases,
    Checked,
}

const COLUMNS: &[(&str, Kind)] = &[
    ("provider", Kind::Text(|p| Some(p.provider))),
    ("model", Kind::Text(|p| Some(p.model))),
    ("aliases", Kind::Aliases),
    ("status", Kind::Text(|p| Some(p.status))),
    ("input", Kind::Usd(|p| p.input)),
    ("output", Kind::Usd(|p| p.output)),
    ("cache_read", Kind::Usd(|p| p.cache_read)),
    ("cache_write_5m", Kind::Usd(|p| p.cache_write("5m"))),
    ("cache_write_1h", Kind::Usd(|p| p.cache_write("1h"))),
    ("cache_write_30m", Kind::Usd(|p| p.cache_write("30m"))),
    ("cache_write_12h", Kind::Usd(|p| p.cache_write("12h"))),
    ("batch_input", Kind::Usd(|p| p.batch_input)),
    ("batch_output", Kind::Usd(|p| p.batch_output)),
    (
        "long_context_threshold",
        Kind::Tokens(|p| {
            p.long_context
                .and_then(|lc| i64::try_from(lc.threshold_tokens).ok())
        }),
    ),
    (
        "long_context_input",
        Kind::Usd(|p| p.long_context.and_then(|lc| lc.rate(Rate::Input))),
    ),
    (
        "long_context_output",
        Kind::Usd(|p| p.long_context.and_then(|lc| lc.rate(Rate::Output))),
    ),
    (
        "long_context_cache_read",
        Kind::Usd(|p| p.long_context.and_then(|lc| lc.rate(Rate::CacheRead))),
    ),
    ("source_url", Kind::Text(|p| Some(p.source_url))),
    ("checked", Kind::Checked),
];

/// Days since 1970-01-01 for an ISO `YYYY-MM-DD` date (Howard Hinnant's `days_from_civil`).
fn days_from_iso(date: &str) -> Option<i32> {
    let mut parts = date.splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i32::try_from(era * 146_097 + doe - 719_468).ok()
}

fn logical(context: &Context, kind: Kind) -> Result<LogicalType> {
    match kind {
        Kind::Text(_) => String::logical_type(context),
        Kind::Usd(_) => f64::logical_type(context),
        Kind::Tokens(_) => i64::logical_type(context),
        Kind::Aliases => <List<String>>::logical_type(context),
        Kind::Checked => DateValue::logical_type(context),
    }
}

struct PricesTable;

impl TableFunctionCallbacks for PricesTable {
    type BindData = ();
    /// The next row to emit.
    type GlobalState = AtomicUsize;
    type LocalState = ();

    fn bind(
        &self,
        context: &Context,
        _arguments: Vec<BindArgument>,
        bind_handle: BindFunctionHandle<'_>,
    ) -> Result<Self::BindData> {
        for (name, kind) in COLUMNS {
            bind_handle.add_result_column(name, &logical(context, *kind)?)?;
        }
        bind_handle.set_cardinality(prices::all().len(), true)?;
        Ok(())
    }

    fn init_global_state(
        &self,
        _context: &Context,
        _bind_data: &Self::BindData,
        _column_data: InitColumnData<'_>,
    ) -> Result<Option<Self::GlobalState>> {
        Ok(Some(AtomicUsize::new(0)))
    }

    fn init_local_state(
        &self,
        _context: &Context,
        _bind_data: &Self::BindData,
        _global_state: Option<&Self::GlobalState>,
        _column_data: InitColumnData<'_>,
    ) -> Result<Option<Self::LocalState>> {
        Ok(None)
    }

    fn exec(
        &self,
        _context: &Context,
        _bind_data: &Self::BindData,
        _local_state: Option<&mut Self::LocalState>,
        global_state: Option<&Self::GlobalState>,
        mut output: DataChunkRef<'_>,
        _columns: ExecColumnInfo<'_>,
    ) -> Result<()> {
        let all = prices::all();
        let cursor =
            global_state.ok_or_else(|| Error::internal("llm_prices: missing scan state"))?;
        let capacity = output.capacity()?;
        let start = cursor.fetch_add(capacity, Ordering::SeqCst);
        if start >= all.len() {
            return Ok(());
        }
        let batch = &all[start..all.len().min(start + capacity)];

        for (position, (_, kind)) in COLUMNS.iter().enumerate() {
            match *kind {
                Kind::Text(get) => {
                    let mut vector = output.get_vector_at_mut::<String>(position)?;
                    vector.set_size(batch.len())?;
                    let mut writer = vector.writer()?;
                    for (row, price) in batch.iter().enumerate() {
                        match get(price) {
                            Some(text) => writer.set(row, text)?,
                            None => writer.set_null(row)?,
                        }
                    }
                }
                Kind::Usd(get) => {
                    let mut vector = output.get_vector_at_mut::<f64>(position)?;
                    vector.set_size(batch.len())?;
                    let mut writer = vector.writer()?;
                    for (row, price) in batch.iter().enumerate() {
                        match get(price) {
                            Some(usd) => writer.set(row, &usd)?,
                            None => writer.set_null(row)?,
                        }
                    }
                }
                Kind::Tokens(get) => {
                    let mut vector = output.get_vector_at_mut::<i64>(position)?;
                    vector.set_size(batch.len())?;
                    let mut writer = vector.writer()?;
                    for (row, price) in batch.iter().enumerate() {
                        match get(price) {
                            Some(tokens) => writer.set(row, &tokens)?,
                            None => writer.set_null(row)?,
                        }
                    }
                }
                Kind::Aliases => {
                    let mut vector = output.get_vector_at_mut::<List<String>>(position)?;
                    vector.set_size(batch.len())?;
                    vector.set_child_size(batch.iter().map(|p| p.aliases.len()).sum())?;
                    let mut writer = vector.list_writer()?;
                    for (row, price) in batch.iter().enumerate() {
                        writer.set_elements(row, price.aliases.iter().map(|a| Some(*a)))?;
                    }
                }
                Kind::Checked => {
                    let mut vector = output.get_vector_at_mut::<DateValue>(position)?;
                    vector.set_size(batch.len())?;
                    let mut writer = vector.writer()?;
                    for (row, price) in batch.iter().enumerate() {
                        match days_from_iso(price.checked) {
                            Some(days) => writer.set(row, &DateValue(days))?,
                            None => writer.set_null(row)?,
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn register(extension: &Extension) -> Result<()> {
    let docs = FunctionDocumentationBuilder::new()
        .description(
            "Every model in the bundled llm-prices table, one row each: provider, model id, aliases, status, \
             and its prices in USD per 1M tokens (input, output, cache read, cache writes per lifetime, batch, \
             long-context tier), with the provider pricing page it was read from and the date it was checked.",
        )
        .add_example("Whole table", "FROM llm_prices()")
        .add_example(
            "Cheapest output per provider",
            "SELECT provider, arg_min(model, output) AS model, min(output) AS usd_per_1m FROM llm_prices() WHERE status = 'listed' GROUP BY provider",
        )
        .add_category(CATEGORY);
    TableFunctionBuilder::new(
        "llm_prices",
        SignatureBuilder::new().documentation(docs),
        PricesTable,
    )
    .register(extension)
}

#[cfg(test)]
mod tests {
    use super::days_from_iso;

    #[test]
    fn iso_dates() {
        assert_eq!(days_from_iso("1970-01-01"), Some(0));
        assert_eq!(days_from_iso("2000-03-01"), Some(11_017));
        assert_eq!(days_from_iso("2026-11-15"), Some(20_772));
        assert_eq!(days_from_iso("2026-13-01"), None);
        assert_eq!(days_from_iso("soon"), None);
    }
}
