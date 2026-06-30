mod common;

use amplify::{s, tiny_s};
use common::{
    default_terms, genesis_seal, stock_with_schema_definition, BENEFICIARY_TXID, CREATED_AT,
};
use rgbstd::containers::{ConsignmentExt, FileContent};
use rgbstd::contract::{FilterIncludeAll, IssuerWrapper, RightsAllocation};
use rgbstd::invoice::Precision;
use rgbstd::stl::{AssetSpec, BridgeLocation, RejectListUrl};
use rgbstd::{ChainNet, Txid};
use schemata::dumb::NoResolver;
use schemata::BridgedFungibleAsset;

fn main() {
    let beneficiary_1 = genesis_seal(BENEFICIARY_TXID, 1, 100_001);
    let beneficiary_2 = genesis_seal(BENEFICIARY_TXID, 2, 100_002);

    let spec = AssetSpec::new("TEST", "Test asset", Precision::CentiMicro);

    let terms = default_terms();

    let reject_list_url = RejectListUrl::from("example.xyz/reject");

    // Address of the token contract on the bridged (external) chain.
    let bridge_location = BridgeLocation::Evm {
        chain_id: 1,
        address: tiny_s!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
    };

    let mut stock =
        stock_with_schema_definition::<BridgedFungibleAsset>("schemata/BridgedFungibleAsset.rgb");

    let contract = stock
        .contract_builder(
            "ssi:anonymous",
            BridgedFungibleAsset::schema().schema_id(),
            ChainNet::BitcoinTestnet4,
        )
        .unwrap()
        .add_global_state("spec", spec)
        .expect("invalid spec")
        .add_global_state("terms", terms)
        .expect("invalid contract terms")
        .add_global_state("rejectListUrl", reject_list_url)
        .expect("invalid reject list url")
        .add_global_state("bridgeLocation", bridge_location)
        .expect("invalid bridge location")
        // BFA mints no assets at genesis: `assetOwner` is only assigned by a `mint`
        // transition. Genesis only grants the right to trigger that mint.
        .add_rights("mintRight", beneficiary_1)
        .expect("invalid mint right")
        .add_rights("mintRight", beneficiary_2)
        .expect("invalid mint right")
        .issue_contract_raw(CREATED_AT)
        .expect("contract doesn't fit schema requirements");

    let contract_id = contract.contract_id();

    eprintln!("{contract}");
    contract
        .save_file("test/bfa-example.rgb")
        .expect("unable to save contract");
    contract
        .save_armored("test/bfa-example.rgba")
        .expect("unable to save armored contract");

    stock.import_contract(contract, NoResolver).unwrap();

    // Reading contract state from the stock:
    let contract = stock
        .contract_wrapper::<BridgedFungibleAsset>(contract_id)
        .unwrap();
    eprintln!("\nThe issued contract:");
    eprintln!("{}", serde_json::to_string(&contract.spec()).unwrap());
    eprintln!("bridgeLocation={:?}", contract.bridge_location());

    let mint_rights = contract
        .mint_rights(&FilterIncludeAll)
        .map(|res| res.expect("state read failure"));
    for RightsAllocation { seal, witness, .. } in mint_rights {
        let witness = witness
            .as_ref()
            .map(Txid::to_string)
            .unwrap_or("~".to_owned());
        eprintln!("mintRight owner={seal}, witness={witness}");
    }
    eprintln!("totalSupply={}", contract.total_issued_supply().value());
}
