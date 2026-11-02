// RGB schemas
//
// SPDX-License-Identifier: Apache-2.0
//
// Copyright (C) 2026 RGB-Tools developers. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Bridged Fungible Assets (BFA) schema.
//!
//! Similar to IFA but with two key differences:
//! - The mint right is similar to IFA's inflation rights, but it has no amount limit
//! - In the "mint" transition, the `issuedSupply` global state and the `beforeBlock` metadata are
//!   pushed to the context's external anchors list to be checked before validation is finalized
//! - The "mint" transition consumes its mint right, the "propagate" transition allows to multiply
//!   and distribute allocations of the mint right
//!
//! Also, instead of an `AssetSpec`, the asset nominals are separate global states: the ticker is a
//! [`UnicodeTicker`], so it is not restricted to ASCII alphanumerics like [`Ticker`].
//!
//! [`Ticker`]: rgbstd::stl::Ticker
//!
//! (!) Not safe to use in a production environment!

use aluvm::isa::Instr;
use aluvm::library::{Lib, LibSite};
use amplify::confinement::Confined;
use rgbstd::contract::{
    AssignmentsFilter, ContractData, ContractError, ContractStateRead, FilteredContractState,
    FungibleAllocation, IssuerWrapper, LinkError, LinkableIssuerWrapper, LinkableSchemaWrapper,
    RightsAllocation, SchemaWrapper,
};
use rgbstd::rgbcore::stl::rgb_contract_id_stl;
use rgbstd::schema::{
    AssignmentDetails, FungibleType, GenesisSchema, GlobalStateSchema, Occurrences,
    OwnedStateSchema, Schema, TransitionSchema,
};
use rgbstd::stl::{
    rgb_bridge_stl, rgb_burn_stl, BridgeLocation, BurnReason, ContractTerms, Details, Name,
    RejectListLocation, StandardTypes, UnicodeTicker,
};
use rgbstd::validation::{Scripts, TypeLibs};
use rgbstd::vm::RgbIsa;
use rgbstd::{
    rgbasm, Amount, ContractId, Genesis, GlobalDetails, MetaDetails, Precision, SchemaId,
    TransitionDetails,
};
use strict_types::encoding::StrictDeserialize;

use crate::{
    ERRNO_BURN_MISMATCH, ERRNO_BURN_ZERO, ERRNO_ISSUED_MISMATCH, ERRNO_NON_EQUAL_IN_OUT,
    GS_BRIDGE_LOCATION, GS_BURNED_ASSET, GS_BURN_REASON, GS_DETAILS, GS_ISSUED_SUPPLY,
    GS_LINKED_FROM_CONTRACT, GS_LINKED_TO_CONTRACT, GS_NAME, GS_PRECISION, GS_REJECT_LIST,
    GS_TERMS, GS_TICKER, MS_BEFORE_BLOCK, OS_ASSET, OS_LINK, OS_MINT, TS_BURN, TS_LINK, TS_MINT,
    TS_PROPAGATE, TS_TRANSFER, TS_TRANSFER_LINK,
};

pub const BFA_SCHEMA_ID: SchemaId = SchemaId::from_array([
    0xdc, 0x8d, 0x31, 0x65, 0x51, 0x10, 0x9d, 0x0c, 0x58, 0x56, 0x4e, 0xef, 0x0e, 0x78, 0x79, 0xb8,
    0xdb, 0x50, 0xc9, 0x47, 0xec, 0x99, 0x38, 0x70, 0xe9, 0xd5, 0xfd, 0x9f, 0xe1, 0xaf, 0x6f, 0x9a,
]);

pub(crate) fn bfa_lib_transfer() -> Lib {
    let code = rgbasm! {
        // Check asset sum is preserved
        put     a8[0],ERRNO_NON_EQUAL_IN_OUT;  // set errno
        svs     OS_ASSET;  // verify sum: inputs == outputs
        test;
    };
    Lib::assemble::<Instr<RgbIsa<FilteredContractState>>>(&code)
        .expect("wrong BFA transfer validation script")
}

pub(crate) fn bfa_lib_mint() -> Lib {
    #[allow(clippy::diverging_sub_expression)]
    let code = rgbasm! {
        put     a8[0],ERRNO_ISSUED_MISMATCH;  // set errno
        put     a8[1],0;               // index = 0 (first global state item)
        put     a16[0],0;              // byte offset = 0 for extr
        ldg     GS_ISSUED_SUPPLY,a8[1],s16[0];  // load issuedSupply into s16[0]
        extr    s16[0],a64[0],a16[0]; // extract u64 into a64[0]
        sas     OS_ASSET;  // check sum of asset allocations in output equals issued_supply
        test;
        ldm     MS_BEFORE_BLOCK,s16[1]; // load beforeBlock metadata into s16[1]
        extr    s16[1],a64[1],a16[0]; // extract u64 into a64[1]
        test;
        pma     a64[0],a64[1]; // record external anchor: amount in a64[0], beforeBlock in a64[1]
        test;
        ret;
    };
    Lib::assemble::<Instr<RgbIsa<FilteredContractState>>>(&code)
        .expect("wrong BFA mint validation script")
}

pub(crate) fn bfa_lib_burn() -> Lib {
    // Burn transitions have no assignments: they burn whole allocations, without change
    const LOAD_GLOBAL: u16 = 42;
    const CHECK: u16 = 4;

    const CHECK_START: u16 = LOAD_GLOBAL;

    #[allow(clippy::diverging_sub_expression)]
    let code: Vec<Instr<RgbIsa<FilteredContractState>>> = rgbasm! {
        // 1. LOAD_GLOBAL: load the burned amount into a64[0], defaulting to 0 when absent.
        // GS_BURNED_ASSET is optional since the transition can burn other assignment types
        put     a8[0],ERRNO_BURN_ZERO;  // set errno
        put     a64[0],0;
        cng     GS_BURNED_ASSET,a8[1];  // check if there is a GS_BURNED_ASSET entry
        ifz     a8[1];
        jif     CHECK_START;  // absent -> burned amount is 0

        inv     st0; // restore st0 after ifz check
        put     a8[0],ERRNO_BURN_MISMATCH;  // set errno
        put     a8[1],0;  // index of the global state item to read
        ldg     GS_BURNED_ASSET,a8[1],s16[0];
        put     a16[0],0;

        extr    s16[0],a64[0],a16[0];
        test;
        ifz     a64[0]; // must be nonzero if present
        inv     st0;
        test;   // fail if GS_BURNED_ASSET is present and equal to 0
        // *CHECK_START* jump destination

        // 2. CHECK: whole allocations are burned, so the sum of inputs must equal the burned
        // amount.
        sps     OS_ASSET;
        test;
    };

    let aluvm_script = Lib::assemble::<Instr<RgbIsa<FilteredContractState>>>(&code)
        .expect("wrong burn validation script");
    let script_length = aluvm_script.code_segment().len() as u16;
    assert_eq!(script_length, LOAD_GLOBAL + CHECK);
    aluvm_script
}

fn bfa_standard_types() -> StandardTypes {
    StandardTypes::new([rgb_contract_id_stl(), rgb_bridge_stl(), rgb_burn_stl()])
}

fn bfa_schema() -> Schema {
    let types = bfa_standard_types();

    Schema {
        ffv: zero!(),
        name: tn!("BridgedFungibleAsset"),
        meta_types: tiny_bmap! {
            MS_BEFORE_BLOCK => MetaDetails {
                sem_id: types.get("RGBBridge.BlockNumber"),
                name: fname!("beforeBlock"),
            },
        },
        global_types: tiny_bmap! {
            GS_TICKER => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBBridge.UnicodeTicker")),
                name: fname!("ticker"),
            },
            GS_NAME => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBContract.Name")),
                name: fname!("name"),
            },
            GS_DETAILS => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBContract.Details")),
                name: fname!("details"),
            },
            GS_PRECISION => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBContract.Precision")),
                name: fname!("precision"),
            },
            GS_TERMS => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBContract.ContractTerms")),
                name: fname!("terms"),
            },
            GS_ISSUED_SUPPLY => GlobalDetails {
                global_state_schema: GlobalStateSchema::many(types.get("RGBContract.Amount")),
                name: fname!("issuedSupply"),
            },
            GS_BURNED_ASSET => GlobalDetails {
                global_state_schema: GlobalStateSchema::many(types.get("RGBContract.Amount")),
                name: fname!("burnedAsset"),
            },
            GS_REJECT_LIST => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBBridge.RejectListLocation")),
                name: fname!("rejectList"),
            },
            GS_LINKED_FROM_CONTRACT => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBCommit.ContractId")),
                name: fname!("linkedFromContract"),
            },
            GS_LINKED_TO_CONTRACT => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBCommit.ContractId")),
                name: fname!("linkedToContract"),
            },
            GS_BRIDGE_LOCATION => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBBridge.BridgeLocation")),
                name: fname!("bridgeLocation"),
            },
            GS_BURN_REASON => GlobalDetails {
                global_state_schema: GlobalStateSchema::many(types.get("RGBBurn.BurnReason")),
                name: fname!("burnReason"),
            },
        },
        owned_types: tiny_bmap! {
            OS_ASSET => AssignmentDetails {
                owned_state_schema: OwnedStateSchema::Fungible(FungibleType::Unsigned64Bit),
                name: fname!("assetOwner"),
                default_transition: TS_TRANSFER,
            },
            OS_MINT => AssignmentDetails {
                owned_state_schema: OwnedStateSchema::Declarative,
                name: fname!("mintRight"),
                default_transition: TS_PROPAGATE,
            },
            OS_LINK => AssignmentDetails {
                owned_state_schema: OwnedStateSchema::Declarative,
                name: fname!("linkRight"),
                default_transition: TS_TRANSFER_LINK,
            }
        },
        genesis: GenesisSchema {
            metadata: none!(),
            globals: tiny_bmap! {
                GS_TICKER => Occurrences::Once,
                GS_NAME => Occurrences::Once,
                GS_DETAILS => Occurrences::NoneOrOnce,
                GS_PRECISION => Occurrences::Once,
                GS_TERMS => Occurrences::Once,
                GS_REJECT_LIST => Occurrences::NoneOrOnce,
                GS_LINKED_FROM_CONTRACT => Occurrences::NoneOrOnce,
                GS_BRIDGE_LOCATION => Occurrences::Once,
            },
            assignments: tiny_bmap! {
                // NOTE: no initial assignments, all new allocations come from mint
                OS_MINT => Occurrences::OnceOrMore,
                OS_LINK => Occurrences::NoneOrOnce,
            },
            validator: None,
        },
        transitions: tiny_bmap! {
            TS_TRANSFER => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: none!(),
                    globals: none!(),
                    inputs: tiny_bmap! {
                        OS_ASSET => Occurrences::NoneOrMore,
                    },
                    assignments: tiny_bmap! {
                        OS_ASSET => Occurrences::NoneOrMore,
                    },
                    validator: Some(LibSite::with(0, bfa_lib_transfer().id())),
                },
                name: fname!("transfer"),
            },
            TS_TRANSFER_LINK => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: none!(),
                    globals: none!(),
                    inputs: tiny_bmap! {
                        OS_LINK => Occurrences::Once,
                    },
                    assignments: tiny_bmap! {
                        OS_LINK => Occurrences::Once,
                    },
                    validator: None,
                },
                name: fname!("transferLink"),
            },
            TS_PROPAGATE => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: none!(),
                    globals: none!(),
                    inputs: tiny_bmap! {
                        OS_MINT => Occurrences::Once,
                    },
                    assignments: tiny_bmap! {
                        OS_MINT => Occurrences::OnceOrMore,
                    },
                    validator: None,
                },
                name: fname!("propagate"),
            },
            TS_MINT => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: tiny_bset!(MS_BEFORE_BLOCK),
                    globals: tiny_bmap! {
                        GS_ISSUED_SUPPLY => Occurrences::Once,
                    },
                    inputs: tiny_bmap! {
                        OS_MINT => Occurrences::Once,
                    },
                    assignments: tiny_bmap! {
                        OS_ASSET => Occurrences::OnceOrMore,
                    },
                    validator: Some(LibSite::with(0, bfa_lib_mint().id())),
                },
                name: fname!("mint"),
            },
            TS_BURN => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: none!(),
                    globals: tiny_bmap! {
                        GS_BURNED_ASSET => Occurrences::NoneOrOnce,
                        GS_BURN_REASON => Occurrences::NoneOrOnce,
                    },
                    inputs: tiny_bmap! {
                        OS_ASSET => Occurrences::NoneOrMore,
                        OS_MINT => Occurrences::NoneOrMore,
                        OS_LINK => Occurrences::NoneOrOnce,
                    },
                    assignments: none!(),
                    validator: Some(LibSite::with(0, bfa_lib_burn().id()))
                },
                name: fname!("burn"),
            },
            TS_LINK => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: none!(),
                    globals: tiny_bmap! {
                        GS_LINKED_TO_CONTRACT => Occurrences::Once,
                    },
                    inputs: tiny_bmap! {
                        OS_LINK => Occurrences::Once,
                    },
                    assignments: none!(),
                    validator: None,
                },
                name: fname!("link"),
            },
        },
        default_assignment: Some(OS_ASSET),
    }
}

#[derive(Default)]
pub struct BridgedFungibleAsset;

impl IssuerWrapper for BridgedFungibleAsset {
    type Wrapper<S: ContractStateRead> = BfaWrapper<S>;

    fn schema() -> Schema { bfa_schema() }

    fn libs() -> TypeLibs { bfa_standard_types().libs() }

    fn scripts() -> Scripts {
        let alu_lib_transfer = bfa_lib_transfer();
        let alu_lib_mint = bfa_lib_mint();
        let alu_lib_burn = bfa_lib_burn();
        Confined::from_checked(bmap! {
            alu_lib_transfer.id() => alu_lib_transfer,
            alu_lib_mint.id() => alu_lib_mint,
            alu_lib_burn.id() => alu_lib_burn,
        })
    }
}

impl LinkableIssuerWrapper for BridgedFungibleAsset {
    type Wrapper<S: ContractStateRead> = BfaWrapper<S>;
}

/// Error extracting the bridge location from a BFA genesis.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Display, Error)]
#[display(doc_comments)]
pub enum BridgeLocationError {
    /// genesis has no `bridgeLocation` global state
    Missing,
    /// genesis global state `bridgeLocation` is not a valid bridge location
    Invalid,
}

impl BridgedFungibleAsset {
    /// The bridge location a BFA contract commits to in `genesis`.
    ///
    /// Meant for consignments which are still under validation, where no contract state is
    /// available yet for [`BfaWrapper::bridge_location`].
    pub fn bridge_location(genesis: &Genesis) -> Result<BridgeLocation, BridgeLocationError> {
        let data = genesis
            .globals
            .get(&GS_BRIDGE_LOCATION)
            .and_then(|values| values.first())
            .ok_or(BridgeLocationError::Missing)?;
        BridgeLocation::from_strict_serialized::<{ u16::MAX as usize }>(data.clone().into())
            .map_err(|_| BridgeLocationError::Invalid)
    }
}

#[derive(Clone, Eq, PartialEq, Debug, From)]
pub struct BfaWrapper<S: ContractStateRead>(ContractData<S>);

impl<S: ContractStateRead> SchemaWrapper<S> for BfaWrapper<S> {
    fn with(data: ContractData<S>) -> Self {
        if data.schema().schema_id() != BFA_SCHEMA_ID {
            panic!("the provided schema is not BFA");
        }
        Self(data)
    }
}

impl<S: ContractStateRead> BfaWrapper<S> {
    pub fn ticker(&self) -> UnicodeTicker {
        let strict_val = &self
            .0
            .global("ticker")
            .next()
            .expect("BFA requires global state `ticker` to have at least one item");
        UnicodeTicker::from_strict_val_unchecked(strict_val)
    }

    pub fn name(&self) -> Name {
        let strict_val = &self
            .0
            .global("name")
            .next()
            .expect("BFA requires global state `name` to have at least one item");
        Name::from_strict_val_unchecked(strict_val)
    }

    pub fn details(&self) -> Option<Details> {
        self.0
            .global("details")
            .next()
            .map(|strict_val| Details::from_strict_val_unchecked(&strict_val))
    }

    pub fn precision(&self) -> Precision {
        let strict_val = &self
            .0
            .global("precision")
            .next()
            .expect("BFA requires global state `precision` to have at least one item");
        Precision::from_strict_val_unchecked(strict_val)
    }

    pub fn contract_terms(&self) -> ContractTerms {
        let strict_val = &self
            .0
            .global("terms")
            .next()
            .expect("BFA requires global state `terms` to have at least one item");
        ContractTerms::from_strict_val_unchecked(strict_val)
    }

    pub fn reject_list(&self) -> Option<RejectListLocation> {
        self.0
            .global("rejectList")
            .next()
            .map(|strict_val| RejectListLocation::from_strict_val_unchecked(&strict_val))
    }

    pub fn total_issued_supply(&self) -> Amount {
        self.0
            .global("issuedSupply")
            .map(|amount| Amount::from_strict_val_unchecked(&amount))
            .sum()
    }

    fn burned_asset(&self) -> impl Iterator<Item = Amount> + '_ {
        self.0
            .global("burnedAsset")
            .map(|amount| Amount::from_strict_val_unchecked(&amount))
    }

    /// Amounts burned by each burn transition, in contract state order.
    pub fn burn_amounts(&self) -> Vec<Amount> { self.burned_asset().collect() }

    /// Total amount burned over the contract's whole history.
    pub fn total_burned(&self) -> Amount { self.burned_asset().sum() }

    /// Reasons committed to by burn transitions, in contract state order.
    pub fn burn_reasons(&self) -> impl Iterator<Item = BurnReason> + '_ {
        self.0
            .global("burnReason")
            .map(|strict_val| BurnReason::from_strict_val_unchecked(&strict_val))
    }

    pub fn allocations<'c>(
        &'c self,
        filter: impl AssignmentsFilter + 'c,
    ) -> impl Iterator<Item = Result<FungibleAllocation, ContractError>> + 'c {
        self.0.fungible_raw(OS_ASSET, filter)
    }

    pub fn mint_rights<'c>(
        &'c self,
        filter: impl AssignmentsFilter + 'c,
    ) -> impl Iterator<Item = Result<RightsAllocation, ContractError>> + 'c {
        self.0.rights("mintRight", filter)
    }

    pub fn bridge_location(&self) -> BridgeLocation {
        let strict_val = self
            .0
            .global("bridgeLocation")
            .next()
            .expect("BFA requires global state `bridgeLocation` to have one item");
        BridgeLocation::from_strict_val_unchecked(&strict_val)
    }
}

impl<S: ContractStateRead> LinkableSchemaWrapper<S> for BfaWrapper<S> {
    fn link_to(&self) -> Result<Option<ContractId>, LinkError> {
        crate::extract_single_contract_id(self.0.global("linkedToContract"))
    }

    fn link_from(&self) -> Result<Option<ContractId>, LinkError> {
        crate::extract_single_contract_id(self.0.global("linkedFromContract"))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::bfa::bfa_schema;

    #[test]
    fn schema_id() {
        let schema_id = bfa_schema().schema_id();
        eprintln!("{:#04x?}", schema_id.to_byte_array());
        assert_eq!(BFA_SCHEMA_ID, schema_id);
    }
}
