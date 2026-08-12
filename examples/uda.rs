mod common;

use std::fs;

use amplify::confinement::SmallBlob;
use amplify::Wrapper;
use common::{genesis_seal, stock_with_schema_definition, BENEFICIARY_TXID, CREATED_AT};
use rgbstd::bitcoin::hashes::{sha256, Hash};
use rgbstd::containers::{ConsignmentExt, FileContent};
use rgbstd::contract::{DataAllocation, FilterIncludeAll, IssuerWrapper};
use rgbstd::invoice::Precision;
use rgbstd::stl::{
    AssetSpec, Attachment, ContractTerms, EmbeddedMedia, MediaType, RicardianContract, TokenData,
};
use rgbstd::{Allocation, ChainNet, TokenIndex, Txid};
use schemata::dumb::NoResolver;
use schemata::UniqueDigitalAsset;

fn main() {
    let beneficiary = genesis_seal(BENEFICIARY_TXID, 1, 100_001);

    let spec = AssetSpec::new("TEST", "Test uda", Precision::Indivisible);

    let file_bytes = fs::read("LICENSE").unwrap();
    let file_hash = sha256::Hash::hash(&file_bytes);
    let terms = ContractTerms {
        text: RicardianContract::default(),
        media: Some(Attachment {
            ty: MediaType::with("text/*"),
            digest: file_hash.to_byte_array().into(),
        }),
    };

    let index = TokenIndex::from_inner(2);
    let preview = EmbeddedMedia {
        ty: MediaType::with("image/*"),
        data: SmallBlob::try_from_iter(vec![0, 0]).expect("invalid data"),
    };
    let token_data = TokenData {
        index,
        preview: Some(preview),
        ..Default::default()
    };

    let allocation = Allocation::with(index, 1);

    let mut stock =
        stock_with_schema_definition::<UniqueDigitalAsset>("schemata/UniqueDigitalAsset.rgb");

    let contract = stock
        .contract_builder(
            "ssi:anonymous",
            UniqueDigitalAsset::schema().schema_id(),
            ChainNet::BitcoinTestnet4,
        )
        .unwrap()
        .add_global_state("spec", spec)
        .expect("invalid spec")
        .add_global_state("terms", terms)
        .expect("invalid contract terms")
        .add_global_state("tokens", token_data)
        .expect("invalid token data")
        .add_data("assetOwner", beneficiary, allocation)
        .expect("invalid asset blob")
        .issue_contract_raw(CREATED_AT)
        .expect("contract doesn't fit schema requirements");

    let contract_id = contract.contract_id();

    eprintln!("{contract}");
    contract
        .save_file("test/uda-example.rgb")
        .expect("unable to save contract");
    contract
        .save_armored("test/uda-example.rgba")
        .expect("unable to save armored contract");

    stock.import_contract(contract, NoResolver).unwrap();

    // Reading contract state from the stock:
    let contract = stock
        .contract_wrapper::<UniqueDigitalAsset>(contract_id)
        .unwrap();
    let allocations = contract
        .allocations(&FilterIncludeAll)
        .map(|res| res.expect("state read failure"));
    eprintln!("\nThe issued contract:");
    eprintln!("{}", serde_json::to_string(&contract.spec()).unwrap());

    for DataAllocation {
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
        eprintln!("state={state}, owner={seal}, witness={witness}");
    }
}
