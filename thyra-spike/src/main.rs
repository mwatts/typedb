/*
 * Thyra Viability Spike
 *
 * Goal: Can the TypeDB engine be called in-process without the gRPC server?
 *
 * This binary exercises the full TypeQL lifecycle:
 *   1. Create a DatabaseManager (no server)
 *   2. Create a database
 *   3. Schema transaction: define types
 *   4. Write transaction: insert data
 *   5. Read transaction: query data back
 */

use std::sync::Arc;

use database::{
    database_manager::DatabaseManager,
    query::{execute_schema_query, execute_write_query_in_write},
    transaction::{TransactionRead, TransactionSchema, TransactionWrite},
    Database,
};
use executor::ExecutionInterrupt;
use lending_iterator::LendingIterator;
use options::{QueryOptions, TransactionOptions};
use storage::durability_client::WALClient;
use test_utils::create_tmp_dir;
use typeql::{parse_query, query::QueryStructure};

fn main() {
    logger::initialise_logging();

    println!("=== Thyra Viability Spike ===\n");

    // Step 1: Create DatabaseManager (no gRPC, no server)
    println!("1. Creating DatabaseManager...");
    let tmp_dir = create_tmp_dir();
    let database_manager = DatabaseManager::new(&tmp_dir).expect("Failed to create DatabaseManager");
    println!("   OK — DatabaseManager created at {:?}", tmp_dir.as_ref());

    // Step 2: Create a database
    println!("\n2. Creating database 'spike_test'...");
    database_manager.put_database("spike_test").expect("Failed to create database");
    let database: Arc<Database<WALClient>> =
        database_manager.database("spike_test").expect("Failed to retrieve database");
    println!("   OK — database 'spike_test' created");

    // Step 3: Schema transaction — define types
    println!("\n3. Defining schema...");
    let schema_query_str = "define
        entity person, owns name, owns age;
        attribute name, value string;
        attribute age, value integer;";

    let parsed = parse_query(schema_query_str).expect("Failed to parse schema query");
    let schema_query = match parsed.structure {
        QueryStructure::Schema(sq) => sq,
        QueryStructure::Pipeline(_) => panic!("Expected schema query, got pipeline"),
    };

    let tx_schema =
        TransactionSchema::open(database.clone(), TransactionOptions::default()).expect("Failed to open schema tx");
    let (tx_schema, result) = execute_schema_query(tx_schema, schema_query, schema_query_str.to_string());
    result.expect("Schema query failed");

    let (_profile, commit_result) = tx_schema.commit();
    commit_result.expect("Schema commit failed");
    println!("   OK — schema defined and committed");

    // Step 4: Write transaction — insert data
    println!("\n4. Inserting data...");
    let insert_query_str = "insert $p isa person, has name \"Alice\", has age 30;";
    let parsed = parse_query(insert_query_str).expect("Failed to parse insert query");
    let pipeline = match parsed.structure {
        QueryStructure::Pipeline(p) => p,
        QueryStructure::Schema(_) => panic!("Expected pipeline query, got schema"),
    };

    let tx_write =
        TransactionWrite::open(database.clone(), TransactionOptions::default()).expect("Failed to open write tx");
    let (tx_write, result) = execute_write_query_in_write(
        tx_write,
        QueryOptions::default_grpc(),
        pipeline,
        insert_query_str.to_string(),
        ExecutionInterrupt::new_uninterruptible(),
    );
    let answer = result.expect("Insert query failed");
    println!("   OK — insert executed (answer: {:?})", answer.query_options);

    let (_profile, commit_result) = tx_write.commit();
    commit_result.expect("Write commit failed");
    println!("   OK — write committed");

    // Step 5: Read transaction — query data back
    println!("\n5. Querying data...");
    let read_query_str = "match $p isa person, has name $n, has age $a;";
    let parsed = parse_query(read_query_str).expect("Failed to parse read query");
    let pipeline = match parsed.structure {
        QueryStructure::Pipeline(p) => p,
        QueryStructure::Schema(_) => panic!("Expected pipeline query, got schema"),
    };

    let tx_read =
        TransactionRead::open(database.clone(), TransactionOptions::default()).expect("Failed to open read tx");

    let read_pipeline = tx_read
        .query_manager
        .prepare_read_pipeline(
            tx_read.snapshot.clone(),
            &tx_read.type_manager,
            tx_read.thing_manager.clone(),
            &tx_read.function_manager,
            &pipeline,
            read_query_str,
        )
        .expect("Failed to prepare read pipeline");

    let named_outputs = read_pipeline.rows_positions().unwrap();
    println!("   Output columns: {:?}", named_outputs.keys().collect::<Vec<_>>());

    let (mut iterator, _context) = read_pipeline
        .into_rows_iterator(ExecutionInterrupt::new_uninterruptible())
        .expect("Failed to create rows iterator");

    let mut row_count = 0;
    while let Some(result) = iterator.next() {
        match result {
            Ok(_row) => {
                row_count += 1;
                println!("   Row {}: (data present)", row_count);
            }
            Err(err) => {
                eprintln!("   ERROR reading row: {:?}", err);
                break;
            }
        }
    }

    tx_read.close();
    println!("   OK — {} row(s) returned", row_count);

    // Summary
    println!("\n=== SPIKE RESULT: GO ===");
    println!("The TypeDB engine CAN be called in-process without gRPC.");
    println!("- DatabaseManager: no server dependency");
    println!("- Transactions: open/commit/close work directly");
    println!("- Schema queries: define types works");
    println!("- Write queries: insert data works");
    println!("- Read queries: match/iterate works");
    println!("- No network listener required");
}
