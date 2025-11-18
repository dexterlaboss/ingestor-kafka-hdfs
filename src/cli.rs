use crate::ledger_storage::{FilterTxIncludeExclude, LedgerCacheConfig, UploaderConfig};
use {
    clap::{value_t_or_exit, values_t, App, Arg, ArgMatches},
    solana_clap_utils::input_validators::{is_parsable, is_pubkey, is_within_range},
    solana_sdk::pubkey::Pubkey,
};

const EXCLUDE_TX_FULL_ADDR: &str = "filter-tx-full-exclude-addr";
const INCLUDE_TX_FULL_ADDR: &str = "filter-tx-full-include-addr";

const EXCLUDE_TX_BY_ADDR_ADDR: &str = "filter-tx-by-addr-exclude-addr";
const INCLUDE_TX_BY_ADDR_ADDR: &str = "filter-tx-by-addr-include-addr";

pub fn block_uploader_app<'a>(version: &'a str) -> App<'a, 'a> {
    return App::new("solana-block-uploader-service")
        .about("Solana Block Uploader Service")
        .version(version)
        .arg(
            Arg::with_name("add_empty_tx_metadata_if_missing")
                .long("add-empty-tx-metadata-if-missing")
                .takes_value(false)
                .help("Add empty transaction metadata if it is missing in input"),
        )
        .arg(
            Arg::with_name("write_block_entries")
                .long("write-block-entries")
                .takes_value(false)
                .help("Write block Entries summaries to HBase 'entries' table."),
        )
        .arg(
            Arg::with_name("validate_only")
                .long("validate-only")
                .takes_value(false)
                .help("Validate/parse stdin JSON only; do not connect to HBase/HDFS or write anything."),
        )
        .arg(
            Arg::with_name("disable_tx")
                .long("disable-tx")
                .takes_value(false)
                .help("Enable historical transaction info over JSON RPC, \
                       including the 'getConfirmedBlock' API."),
        )
        .arg(
            Arg::with_name("disable_tx_by_addr")
                .long("disable-tx-by-addr")
                .takes_value(false)
                .help("Enable historical transaction info over JSON RPC, \
                       including the 'getConfirmedBlock' API."),
        )
        .arg(
            Arg::with_name("disable_blocks")
                .long("disable-blocks")
                .takes_value(false)
                .help("Enable historical transaction info over JSON RPC, \
                       including the 'getConfirmedBlock' API."),
        )
        .arg(
            Arg::with_name("enable_full_tx")
                .long("enable-full-tx")
                .takes_value(false)
                .help("Enable historical transaction info over JSON RPC, \
                       including the 'getConfirmedBlock' API."),
        )
        .arg(
            Arg::with_name("use_md5_row_key_salt")
                .long("use-md5-row-key-salt")
                .takes_value(false)
                .help("Add md5 salt to blocks table row keys."),
        )
        .arg(
            Arg::with_name("hash_tx_full_row_keys")
                .long("hash-tx-full-row-keys")
                .takes_value(false)
                .help("Hash tx_full table row keys."),
        )
        .arg(
            Arg::with_name("filter_tx_by_addr_programs")
                .long("filter-tx-by-addr-programs")
                .takes_value(false)
                .help("Skip program accounts from tx-by-addr index."),
        )
        .arg(
            Arg::with_name("filter_tx_by_addr_readonly_accounts")
                .long("filter-tx-by-addr-readonly-accounts")
                .takes_value(false)
                .help("Skip readonly accounts from tx-by-addr index."),
        )
        .arg(
            Arg::with_name("filter_tx_voting")
                .long("filter-tx-voting")
                .takes_value(false)
                .help("Do not store voting transactions in tx."),
        )
        .arg(
            Arg::with_name("filter_tx_by_addr_voting")
                .long("filter-tx-by-addr-voting")
                .takes_value(false)
                .help("Do not store voting transactions in tx-by-addr."),
        )
        .arg(
            Arg::with_name("filter_tx_full_voting")
                .long("filter-tx-full-voting")
                .takes_value(false)
                .help("Do not store voting transactions in tx_full."),
        )
        .arg(
            Arg::with_name("filter_all_voting")
                .long("filter-all-voting")
                .takes_value(false)
                .help("Do not store voting transactions in tx, tx-by-addr and tx_full."),
        )
        .arg(
            Arg::with_name("filter_tx_error")
                .long("filter-tx-error")
                .takes_value(false)
                .help("Do not store failed transactions in tx-by-addr and tx_full."),
        )
        .arg(
            Arg::with_name("filter_tx_by_addr_error")
                .long("filter-tx-by-addr-error")
                .takes_value(false)
                .help("Do not store failed transactions in tx-by-addr and tx_full."),
        )
        .arg(
            Arg::with_name("filter_tx_full_error")
                .long("filter-tx-full-error")
                .takes_value(false)
                .help("Do not store failed transactions in tx-by-addr and tx_full."),
        )
        .arg(
            Arg::with_name("filter_all_error")
                .long("filter-all-error")
                .takes_value(false)
                .help("Do not store failed transactions in tx, tx-by-addr and tx_full."),
        )
        .arg(
            Arg::with_name("disable_blocks_compression")
                .long("disable-blocks-compression")
                .takes_value(false)
                .help("Disables blocks table compression."),
        )
        .arg(
            Arg::with_name("disable_tx_compression")
                .long("disable-tx-compression")
                .takes_value(false)
                .help("Disables tx table compression."),
        )
        .arg(
            Arg::with_name("disable_tx_by_addr_compression")
                .long("disable-tx-by-addr-compression")
                .takes_value(false)
                .help("Disables tx-by-addr table compression."),
        )
        .arg(
            Arg::with_name("disable_tx_full_compression")
                .long("disable-tx-full-compression")
                .takes_value(false)
                .help("Disables tx-full table compression."),
        )
        .arg(
            Arg::with_name("filter_tx_full_include_addr")
                .long(INCLUDE_TX_FULL_ADDR)
                .takes_value(true)
                .validator(is_pubkey)
                .multiple(true)
                .value_name("KEY")
                .help("Store only transactions with this account key in tx-full."),
        )
        .arg(
            Arg::with_name("filter_tx_full_exclude_addr")
                .long(EXCLUDE_TX_FULL_ADDR)
                .takes_value(true)
                .validator(is_pubkey)
                .conflicts_with("filter_tx_full_include_addr")
                .multiple(true)
                .value_name("KEY")
                .help("Store all transactions in tx-full except the ones with this account key. Overrides filter_tx_full_include_addr."),
        )
        .arg(
            Arg::with_name("filter_tx_by_addr_include_addr")
                .long(INCLUDE_TX_BY_ADDR_ADDR)
                .takes_value(true)
                .validator(is_pubkey)
                .multiple(true)
                .value_name("KEY")
                .help("Store only transactions with this account key in tx-by-addr."),
        )
        .arg(
            Arg::with_name("filter_tx_by_addr_exclude_addr")
                .long(EXCLUDE_TX_BY_ADDR_ADDR)
                .takes_value(true)
                .validator(is_pubkey)
                .conflicts_with("filter_tx_by_addr_include_addr")
                .multiple(true)
                .value_name("KEY")
                .help("Store all transactions in tx-by-addr except the ones with this account key. Overrides filter_tx_by_addr_include_addr."),
        )
        .arg(
            Arg::with_name("enable_full_tx_cache")
                .long("enable-full-tx-cache")
                .takes_value(false)
                .help("Enable block transaction cache."),
        )
        .arg(
            Arg::with_name("cache_timeout")
                .long("cache-timeout")
                .value_name("SECONDS")
                .validator(is_parsable::<u64>)
                .takes_value(true)
                .help("Cache connection timeout"),
        )
        .arg(
            Arg::with_name("tx_cache_expiration")
                .long("tx-cache-expiration")
                .value_name("DAYS")
                .validator(|v| is_within_range::<usize, _>(v, 0..=30))
                .takes_value(true)
                .help("Number of days before tx cache records expire"),
        )
        .arg(
            Arg::with_name("cache_address")
                .long("cache-address")
                .value_name("ADDRESS")
                .takes_value(true)
                .help("Address of the cache server"),
        )
        .arg(
            Arg::with_name("hbase_skip_wal")
                .long("hbase-skip-wal")
                .takes_value(false)
                .help("If HBase should skip WAL when writing new data."),
        )
    ;
}

/// Process uploader-related CLI arguments
pub fn process_uploader_arguments(matches: &ArgMatches) -> UploaderConfig {
    let write_block_entries = matches.is_present("write_block_entries");
    let disable_tx = matches.is_present("disable_tx");
    let disable_tx_by_addr = matches.is_present("disable_tx_by_addr");
    let disable_blocks = matches.is_present("disable_blocks");
    let enable_full_tx = matches.is_present("enable_full_tx");
    let use_md5_row_key_salt = matches.is_present("use_md5_row_key_salt");
    let hash_tx_full_row_keys = matches.is_present("hash_tx_full_row_keys");
    let filter_program_accounts = matches.is_present("filter_tx_by_addr_programs");
    let filter_readonly_accounts = matches.is_present("filter_tx_by_addr_readonly_accounts");
    let filter_tx_voting = matches.is_present("filter_tx_voting");
    let filter_tx_by_addr_voting = matches.is_present("filter_tx_by_addr_voting");
    let filter_tx_full_voting = matches.is_present("filter_tx_full_voting");
    let filter_all_voting = matches.is_present("filter_all_voting");
    let filter_tx_error = matches.is_present("filter_tx_error");
    let filter_tx_by_addr_error = matches.is_present("filter_tx_by_addr_error");
    let filter_tx_full_error = matches.is_present("filter_tx_full_error");
    let filter_all_error = matches.is_present("filter_all_error");
    let use_blocks_compression = !matches.is_present("disable_blocks_compression");
    let use_tx_compression = !matches.is_present("disable_tx_compression");
    let use_tx_by_addr_compression = !matches.is_present("disable_tx_by_addr_compression");
    let use_tx_full_compression = !matches.is_present("disable_tx_full_compression");
    let hbase_write_to_wal = !matches.is_present("hbase_skip_wal");

    let filter_tx_full_include_addrs: std::collections::HashSet<Pubkey> =
        values_t!(matches, "filter_tx_full_include_addr", Pubkey)
            .unwrap_or_default()
            .iter()
            .cloned()
            .collect();

    let filter_tx_full_exclude_addrs: std::collections::HashSet<Pubkey> =
        values_t!(matches, "filter_tx_full_exclude_addr", Pubkey)
            .unwrap_or_default()
            .iter()
            .cloned()
            .collect();

    let filter_tx_by_addr_include_addrs: std::collections::HashSet<Pubkey> =
        values_t!(matches, "filter_tx_by_addr_include_addr", Pubkey)
            .unwrap_or_default()
            .iter()
            .cloned()
            .collect();

    let filter_tx_by_addr_exclude_addrs: std::collections::HashSet<Pubkey> =
        values_t!(matches, "filter_tx_by_addr_exclude_addr", Pubkey)
            .unwrap_or_default()
            .iter()
            .cloned()
            .collect();

    let tx_full_filter = create_filter(filter_tx_full_exclude_addrs, filter_tx_full_include_addrs);
    let tx_by_addr_filter = create_filter(
        filter_tx_by_addr_exclude_addrs,
        filter_tx_by_addr_include_addrs,
    );

    UploaderConfig {
        write_block_entries,
        tx_full_filter,
        tx_by_addr_filter,
        disable_tx,
        disable_tx_by_addr,
        disable_blocks,
        enable_full_tx,
        use_md5_row_key_salt,
        hash_tx_full_row_keys,
        filter_program_accounts,
        filter_readonly_accounts,
        filter_tx_voting,
        filter_tx_by_addr_voting,
        filter_tx_full_voting,
        filter_all_voting,
        filter_tx_error,
        filter_tx_by_addr_error,
        filter_tx_full_error,
        filter_all_error,
        use_blocks_compression,
        use_tx_compression,
        use_tx_by_addr_compression,
        use_tx_full_compression,
        hbase_write_to_wal,
        ..Default::default()
    }
}

/// Process cache-related CLI arguments
pub fn process_cache_arguments(matches: &ArgMatches) -> LedgerCacheConfig {
    let enable_full_tx_cache = matches.is_present("enable_full_tx_cache");

    let address = if matches.is_present("cache_address") {
        value_t_or_exit!(matches, "cache_address", String)
    } else {
        String::new()
    };

    let timeout = if matches.is_present("cache_timeout") {
        Some(std::time::Duration::from_secs(value_t_or_exit!(
            matches,
            "cache_timeout",
            u64
        )))
    } else {
        None
    };

    let tx_cache_expiration = if matches.is_present("tx_cache_expiration") {
        Some(std::time::Duration::from_secs(
            value_t_or_exit!(matches, "tx_cache_expiration", u64) * 24 * 60 * 60,
        ))
    } else {
        None
    };

    LedgerCacheConfig {
        enable_full_tx_cache,
        address,
        timeout,
        tx_cache_expiration,
        ..Default::default()
    }
}

/// Helper function to create a filter
fn create_filter(
    filter_tx_exclude_addrs: std::collections::HashSet<Pubkey>,
    filter_tx_include_addrs: std::collections::HashSet<Pubkey>,
) -> Option<FilterTxIncludeExclude> {
    let exclude_tx_addrs = !filter_tx_exclude_addrs.is_empty();
    let include_tx_addrs = !filter_tx_include_addrs.is_empty();

    if exclude_tx_addrs || include_tx_addrs {
        let filter_tx_addrs = FilterTxIncludeExclude {
            exclude: exclude_tx_addrs,
            addrs: if exclude_tx_addrs {
                filter_tx_exclude_addrs
            } else {
                filter_tx_include_addrs
            },
        };
        Some(filter_tx_addrs)
    } else {
        None
    }
}
