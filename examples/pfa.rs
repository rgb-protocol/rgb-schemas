mod common;

use common::{
    default_terms, genesis_seal, stock_with_schema_definition, BENEFICIARY_TXID, CREATED_AT,
};
use rgbstd::bitcoin::CompressedPublicKey;
use rgbstd::containers::{ConsignmentExt, FileContent};
use rgbstd::contract::{FilterIncludeAll, FungibleAllocation, IssuerWrapper};
use rgbstd::invoice::Precision;
use rgbstd::stl::AssetSpec;
use rgbstd::{Amount, ChainNet, Txid};
use schemata::dumb::NoResolver;
use schemata::PermissionedFungibleAsset;

fn main() {
    let beneficiary = genesis_seal(BENEFICIARY_TXID, 1, 100_001);

    let spec = AssetSpec::new("TEST", "Test asset", Precision::CentiMicro);

    let terms = default_terms();

    let issued_supply = Amount::from(100000u64);

    let pubkey = CompressedPublicKey::from_slice(&[
        2, 199, 163, 211, 116, 75, 108, 119, 241, 66, 54, 236, 233, 189, 142, 108, 37, 135, 56,
        128, 200, 176, 199, 9, 117, 132, 72, 200, 167, 185, 4, 64, 53,
    ])
    .unwrap();

    let mut stock = stock_with_schema_definition::<PermissionedFungibleAsset>(
        "schemata/PermissionedFungibleAsset.rgb",
    );

    let contract = stock
        .contract_builder(
            "ssi:anonymous",
            PermissionedFungibleAsset::schema().schema_id(),
            ChainNet::BitcoinTestnet4,
        )
        .unwrap()
        .add_global_state("spec", spec)
        .expect("invalid spec")
        .add_global_state("terms", terms)
        .expect("invalid contract terms")
        .add_global_state("issuedSupply", issued_supply)
        .expect("invalid issued supply")
        .add_fungible_state("assetOwner", beneficiary, 100000u64)
        .expect("invalid fungible state")
        .add_global_state("pubkey", pubkey)
        .expect("invalid pubkey")
        .issue_contract_raw(CREATED_AT)
        .expect("contract doesn't fit schema requirements");

    let contract_id = contract.contract_id();

    eprintln!("{contract}");
    contract
        .save_file("test/pfa-example.rgb")
        .expect("unable to save contract");
    contract
        .save_armored("test/pfa-example.rgba")
        .expect("unable to save armored contract");

    stock.import_contract(contract, NoResolver).unwrap();

    // Reading contract state from the stock:
    let contract = stock
        .contract_wrapper::<PermissionedFungibleAsset>(contract_id)
        .unwrap();
    let allocations = contract
        .allocations(&FilterIncludeAll)
        .map(|res| res.expect("state read failure"));
    eprintln!("\nThe issued contract:");
    eprintln!("{}", serde_json::to_string(&contract.spec()).unwrap());

    for FungibleAllocation {
        seal,
        state,
        witness,
        ..
    } in allocations
    {
        let witness = witness
            .as_ref()
            .map(Txid::to_string)
            .unwrap_or("~".to_owned());
        eprintln!("amount={}, owner={seal}, witness={witness}", state.value());
    }
    eprintln!("totalSupply={}", contract.total_issued_supply().value());
}
