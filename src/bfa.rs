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
//! - In the "mint" transition, the `issuedSupply` global state and the `afterBlock` metadata are
//!   pushed to the context's external anchors list to be checked before validation is finalized
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
    rgb_bridge_stl, rgb_burn_stl, AssetSpec, BridgeLocation, BurnReason, ContractTerms,
    RejectListUrl, StandardTypes,
};
use rgbstd::validation::{Scripts, TypeLibs};
use rgbstd::vm::RgbIsa;
use rgbstd::{
    rgbasm, Amount, ContractId, Genesis, GlobalDetails, MetaDetails, SchemaId, TransitionDetails,
};
use strict_types::encoding::StrictDeserialize;

use crate::{
    ERRNO_BURN_MISMATCH, ERRNO_BURN_ZERO, ERRNO_HIDDEN_BURN, ERRNO_ISSUED_MISMATCH,
    ERRNO_MISSING_INPUT, ERRNO_NON_EQUAL_IN_OUT, GS_BRIDGE_LOCATION, GS_BURNED_ASSET,
    GS_BURN_REASON, GS_ISSUED_SUPPLY, GS_LINKED_FROM_CONTRACT, GS_LINKED_TO_CONTRACT, GS_NOMINAL,
    GS_REJECT_LIST_URL, GS_TERMS, MS_AFTER_BLOCK, OS_ASSET, OS_LINK, OS_MINT, TS_BURN, TS_LINK,
    TS_MINT, TS_TRANSFER,
};

pub const BFA_SCHEMA_ID: SchemaId = SchemaId::from_array([
    0x4a, 0xb2, 0x1c, 0x87, 0x2b, 0x75, 0x62, 0x4c, 0x36, 0x52, 0xc4, 0xf4, 0x5c, 0x58, 0x26, 0x9b,
    0x1e, 0xd4, 0x8c, 0x5e, 0xcc, 0x02, 0x16, 0x80, 0x6a, 0x70, 0xac, 0x5b, 0x36, 0xc2, 0x8c, 0x63,
]);

pub(crate) fn bfa_lib_transfer() -> Lib {
    let code = rgbasm! {
        // Check asset sum is preserved
        put     a8[0],ERRNO_NON_EQUAL_IN_OUT;  // set errno
        svs     OS_ASSET;  // verify sum: inputs == outputs
        test;

        // Count-based check for link rights
        cnp     OS_LINK,a16[0];  // count link right inputs
        cns     OS_LINK,a16[1];  // count link right outputs
        eq.n    a16[0],a16[1];
        test;

        // Mint rights validation
        cnp     OS_MINT,a16[0];  // count input mint rights
        cns     OS_MINT,a16[1];  // count output mint rights
        // Check if input count is 0
        put     a16[2],0;  // store 0 in a16[2]
        eq.n    a16[0],a16[2];  // check if input_count == 0
        jif     48;  // jump to 0x30 if input_count == 0
        // Input count > 0, check that output count >= input count
        put     a8[0],ERRNO_HIDDEN_BURN;  // set errno
        lt.u    a16[1],a16[0];  // output_count < input_count
        inv     st0;  // output_count >= input_count
        test;  // fail if output_count < input_count
        ret;  // return execution flow
        // 0x30: Input count is 0, output count must also be 0
        put     a8[0],ERRNO_MISSING_INPUT;  // set errno
        eq.n    a16[1],a16[0];  // check if output_count == input_count
        test;  // fail if output_count != input_count (=0)

        ret;
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
        ldm     MS_AFTER_BLOCK,s16[1]; // load afterBlock metadata into s16[1]
        extr    s16[1],a64[1],a16[0]; // extract u64 into a64[1]
        test;
        pma     a64[0],a64[1]; // record external anchor: amount in a64[0], afterBlock in a64[1]
        test;
        ret;
    };
    Lib::assemble::<Instr<RgbIsa<FilteredContractState>>>(&code)
        .expect("wrong BFA mint validation script")
}

pub(crate) fn bfa_lib_burn() -> Lib {
    const BEFORE_LOOP: u16 = 16;
    const LOOP: u16 = 20;
    const LOAD_GLOBAL: u16 = 34;
    const FINAL_CHECKS: u16 = 24;

    const LOOP_STRT: u16 = BEFORE_LOOP;
    const LOOP_EXIT: u16 = LOOP_STRT + LOOP;
    const LDGL_EXIT: u16 = LOOP_EXIT + LOAD_GLOBAL;

    #[allow(clippy::diverging_sub_expression)]
    let code: Vec<Instr<RgbIsa<FilteredContractState>>> = rgbasm! {
        // VALIDATE OS_ASSET BURN
        // 1. BEFORE_LOOP: load sum of OS_ASSET outputs into a64[0]
        put     a8[0],ERRNO_BURN_MISMATCH;  // set errno
        put     a16[0],0; // index
        put     a64[0],0; // accumulator
        cns     OS_ASSET,a16[1]; // count OS_ASSET assignments
        // 2. LOOP: loop through all OS_ASSET assignments and sum their values into a64[0]
        // *LOOP_STRT* jump destination
        eq.n    a16[0],a16[1];
        jif     LOOP_EXIT; // if we cycled all assignments, end loop
        ldf     OS_ASSET,a16[0],a64[1]; // load assignment at the current index
        add.uc  a64[1],a64[0]; // add it to the accumulator
        test;
        inc     a16[0]; // increment index
        jmp     LOOP_STRT; // go to the next iteration
        // *LOOP_EXIT* jump destination

        // 3. LOAD_GLOBAL: load the burned amount into a64[1], defaulting to 0 when absent.
        // GS_BURNED_ASSET is optional since the transition can burn other assignment types
        put     a64[1],0;
        cng     GS_BURNED_ASSET,a8[1];  // check if there is a GS_BURNED_ASSET entry
        ifz     a8[1];
        jif     LDGL_EXIT;  // absent -> burned amount is 0

        inv     st0; // restore st0 after ifz check
        put     a8[1],0;  // index of the global state item to read
        ldg     GS_BURNED_ASSET,a8[1],s16[0];
        put     a16[0],0;

        extr    s16[0],a64[1],a16[0];
        test;
        ifz     a64[1]; // must be nonzero if present
        inv     st0;
        test;   // fail if GS_BURNED_ASSET is present and equal to 0
        // *LDGL_EXIT* jump destination

        // 4. FINAL_CHECKS:
        // - sum of inputs must equal burned amount plus sum of outputs
        add.uc  a64[1],a64[0];
        test;
        sps     OS_ASSET;
        test;
        // - a nonzero amount must be burned (pure transfer is forbidden)
        put     a8[0],ERRNO_BURN_ZERO;  // set errno
        ifz     a64[1];
        st.s    a8[1]; // burned_amount == 0
        ifz     a64[0]; // os_asset input == 0
        inv     st0;
        st.n    a8[1]; // burned_amount == 0 AND os_asset input > 0
        ifz     a8[1];
        test; // fail if burned_amount == 0 AND os_asset input > 0
    };

    let aluvm_script = Lib::assemble::<Instr<RgbIsa<FilteredContractState>>>(&code)
        .expect("wrong burn validation script");
    let script_length = aluvm_script.code_segment().len() as u16;
    assert_eq!(script_length, LDGL_EXIT + FINAL_CHECKS);
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
            MS_AFTER_BLOCK => MetaDetails {
                sem_id: types.get("RGBBridge.BlockNumber"),
                name: fname!("afterBlock"),
            },
        },
        global_types: tiny_bmap! {
            GS_NOMINAL => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBContract.AssetSpec")),
                name: fname!("spec"),
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
            GS_REJECT_LIST_URL => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBContract.RejectListUrl")),
                name: fname!("rejectListUrl"),
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
                // Void (no amount): the right to mint is signalled by mere possession,
                // not by a numeric allowance.
                owned_state_schema: OwnedStateSchema::Declarative,
                name: fname!("mintRight"),
                default_transition: TS_TRANSFER,
            },
            OS_LINK => AssignmentDetails {
                owned_state_schema: OwnedStateSchema::Declarative,
                name: fname!("linkRight"),
                default_transition: TS_TRANSFER,
            }
        },
        genesis: GenesisSchema {
            metadata: none!(),
            globals: tiny_bmap! {
                GS_NOMINAL => Occurrences::Once,
                GS_TERMS => Occurrences::Once,
                GS_REJECT_LIST_URL => Occurrences::NoneOrOnce,
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
                        OS_MINT => Occurrences::NoneOrMore,
                        OS_LINK => Occurrences::NoneOrOnce,
                    },
                    assignments: tiny_bmap! {
                        OS_ASSET => Occurrences::NoneOrMore,
                        OS_MINT => Occurrences::NoneOrMore,
                        OS_LINK => Occurrences::NoneOrOnce,
                    },
                    validator: Some(LibSite::with(0, bfa_lib_transfer().id())),
                },
                name: fname!("transfer"),
            },
            TS_MINT => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: tiny_bset!(MS_AFTER_BLOCK),
                    globals: tiny_bmap! {
                        GS_ISSUED_SUPPLY => Occurrences::Once,
                    },
                    inputs: tiny_bmap! {
                        OS_MINT => Occurrences::Once,
                    },
                    assignments: tiny_bmap! {
                        OS_ASSET => Occurrences::OnceOrMore,
                        OS_MINT => Occurrences::NoneOrOnce,
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
                    assignments: tiny_bmap! {
                        OS_ASSET => Occurrences::NoneOrMore,
                    },
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
    pub fn spec(&self) -> AssetSpec {
        let strict_val = &self
            .0
            .global("spec")
            .next()
            .expect("BFA requires global state `spec` to have at least one item");
        AssetSpec::from_strict_val_unchecked(strict_val)
    }

    pub fn contract_terms(&self) -> ContractTerms {
        let strict_val = &self
            .0
            .global("terms")
            .next()
            .expect("BFA requires global state `terms` to have at least one item");
        ContractTerms::from_strict_val_unchecked(strict_val)
    }

    pub fn reject_list_url(&self) -> Option<RejectListUrl> {
        self.0
            .global("rejectListUrl")
            .next()
            .map(|strict_val| RejectListUrl::from_strict_val_unchecked(&strict_val))
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
