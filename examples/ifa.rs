mod common;

use common::{
    default_terms, genesis_seal, stock_with_schema_definition, BENEFICIARY_TXID, CREATED_AT,
};
use rgbstd::containers::{ConsignmentExt, FileContent};
use rgbstd::contract::{FilterIncludeAll, FungibleAllocation, IssuerWrapper};
use rgbstd::invoice::Precision;
use rgbstd::stl::{AssetSpec, RejectListUrl};
use rgbstd::{Amount, ChainNet, Txid};
use schemata::dumb::NoResolver;
use schemata::InflatableFungibleAsset;

fn main() {
    let beneficiary_1 = genesis_seal(BENEFICIARY_TXID, 1, 100_001);
    let beneficiary_2 = genesis_seal(BENEFICIARY_TXID, 2, 100_002);

    let spec = AssetSpec::new("TEST", "Test asset", Precision::CentiMicro);

    let terms = default_terms();

    let issued_supply = Amount::from(100000u64);

    let max_supply = Amount::from(150000u64);

    let reject_list_url = RejectListUrl::from("example.xyz/reject");

    let mut stock = stock_with_schema_definition::<InflatableFungibleAsset>(
        "schemata/InflatableFungibleAsset.rgb",
    );

    let contract = stock
        .contract_builder(
            "ssi:anonymous",
            InflatableFungibleAsset::schema().schema_id(),
            ChainNet::BitcoinTestnet4,
        )
        .unwrap()
        .add_global_state("spec", spec)
        .expect("invalid spec")
        .add_global_state("terms", terms)
        .expect("invalid contract terms")
        .add_global_state("issuedSupply", issued_supply)
        .expect("invalid issued supply")
        .add_global_state("maxSupply", max_supply)
        .expect("invalid max supply")
        .add_global_state("rejectListUrl", reject_list_url)
        .expect("invalid reject list url")
        .add_fungible_state("assetOwner", beneficiary_1, issued_supply.value())
        .expect("invalid fungible state")
        .add_fungible_state(
            "inflationAllowance",
            beneficiary_2,
            max_supply.value() - issued_supply.value(),
        )
        .expect("invalid fungible state")
        .issue_contract_raw(CREATED_AT)
        .expect("contract doesn't fit schema requirements");

    let contract_id = contract.contract_id();

    eprintln!("{contract}");
    contract
        .save_file("test/ifa-example.rgb")
        .expect("unable to save contract");
    contract
        .save_armored("test/ifa-example.rgba")
        .expect("unable to save armored contract");

    stock.import_contract(contract, NoResolver).unwrap();

    // Reading contract state from the stock:
    let contract = stock
        .contract_wrapper::<InflatableFungibleAsset>(contract_id)
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
