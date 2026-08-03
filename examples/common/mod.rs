use std::str::FromStr;

use rgbstd::containers::FileContent;
use rgbstd::contract::IssuerWrapper;
use rgbstd::persistence::Stock;
use rgbstd::stl::{ContractTerms, RicardianContract};
use rgbstd::txout::BlindSeal;
use rgbstd::validation::SchemaDefinition;
use rgbstd::{GenesisSeal, Txid};

/// Fixed issuance timestamp so example output is reproducible.
pub const CREATED_AT: i64 = 1713261744;

pub const BENEFICIARY_TXID: &str =
    "14295d5bb1a191cdb6286dc0944df938421e3dfcbf0811353ccac4100c2068c5";

#[allow(dead_code)]
pub fn default_terms() -> ContractTerms {
    ContractTerms {
        text: RicardianContract::default(),
        media: None,
    }
}

/// Reads a schema definition off disk. It carries its own type libraries, so
/// nothing has to be supplied from code.
pub fn stock_with_schema_definition<C: IssuerWrapper>(schema_rgb: &str) -> Stock {
    let _ = core::marker::PhantomData::<C>;
    let mut stock = Stock::in_memory();
    let schema_def = SchemaDefinition::load_file(schema_rgb).unwrap();
    stock
        .import_schema_definition(schema_def)
        .expect("invalid schema definition");
    stock
}

pub fn genesis_seal(txid: &str, vout: u32, blinding: u64) -> GenesisSeal {
    GenesisSeal::from(BlindSeal::with_blinding(Txid::from_str(txid).unwrap(), vout, blinding))
}
