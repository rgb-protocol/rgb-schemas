// RGB schemas
//
// SPDX-License-Identifier: Apache-2.0
//
// Copyright (C) 2025-2026 RGB-Tools developers.
//
// Portions of this file are derived from other file(s) of the original
// project, some of which may since have been renamed, moved, or deleted:
//   Copyright (C) 2019-2024 LNP/BP Standards Association. All rights reserved.
//
// Everything else in this file, including all modifications made to
// the derived portions, is copyright RGB-Tools developers as stated
// above.
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

//! Inflatable Fungible Assets (IFA) schema.
//! (!) Not safe to use in a production environment!

use aluvm::isa::Instr;
use aluvm::library::{Lib, LibSite};
use amplify::confinement::Confined;
use rgbstd::contract::{
    AssignmentsFilter, ContractData, ContractError, ContractStateRead, FilteredContractState,
    FungibleAllocation, IssuerWrapper, LinkError, LinkableIssuerWrapper, LinkableSchemaWrapper,
    SchemaWrapper,
};
use rgbstd::rgbcore::stl::rgb_contract_id_stl;
use rgbstd::schema::{
    AssignmentDetails, FungibleType, GenesisSchema, GlobalStateSchema, Occurrences,
    OwnedStateSchema, Schema, TransitionSchema,
};
use rgbstd::stl::{
    rgb_burn_stl, AssetSpec, BurnReason, ContractTerms, RejectListUrl, StandardTypes,
};
use rgbstd::validation::{Scripts, TypeLibs};
use rgbstd::vm::RgbIsa;
use rgbstd::{
    rgbasm, Amount, AssignmentType, ContractId, GlobalDetails, MetaDetails, SchemaId,
    TransitionDetails,
};

use crate::{
    ERRNO_BURN_MISMATCH, ERRNO_BURN_ZERO, ERRNO_INFLATION_EXCEEDS_ALLOWANCE,
    ERRNO_INFLATION_MISMATCH, ERRNO_ISSUED_MISMATCH, ERRNO_NON_EQUAL_IN_OUT, GS_BURNED_ASSET,
    GS_BURNED_INFLATION, GS_BURN_REASON, GS_ISSUED_SUPPLY, GS_LINKED_FROM_CONTRACT,
    GS_LINKED_TO_CONTRACT, GS_MAX_SUPPLY, GS_NOMINAL, GS_REJECT_LIST_URL, GS_TERMS,
    MS_ALLOWED_INFLATION, OS_ASSET, OS_INFLATION, OS_LINK, TS_BURN, TS_INFLATION, TS_LINK,
    TS_TRANSFER,
};

pub const IFA_SCHEMA_ID: SchemaId = SchemaId::from_array([
    0x63, 0x66, 0xba, 0x49, 0x6e, 0x2b, 0x63, 0x78, 0x11, 0xc0, 0xb1, 0x38, 0xd0, 0xec, 0x48, 0x2e,
    0x1c, 0x0e, 0xc8, 0xbd, 0x85, 0x47, 0xda, 0x69, 0xb5, 0xee, 0xfd, 0xd2, 0x82, 0xdb, 0x21, 0xed,
]);

/// Field name of the global state carrying the burned amount for `assignment_type`.
pub fn burn_global_by_assignment(assignment_type: &AssignmentType) -> &'static str {
    match assignment_type {
        &OS_ASSET => "burnedAsset",
        &OS_INFLATION => "burnedInflation",
        a => panic!("unexpected assignment type {a}"),
    }
}

pub(crate) fn ifa_lib_genesis() -> Lib {
    #[allow(clippy::diverging_sub_expression)]
    let code = rgbasm! {
        // Set common offsets
        put     a8[1],0;
        put     a16[0],0;

        // Check reported issued supply against sum of asset allocations in output
        put     a8[0],ERRNO_ISSUED_MISMATCH;  // set errno
        ldg     GS_ISSUED_SUPPLY,a8[1],s16[0];  // read issued supply global state
        extr    s16[0],a64[0],a16[0];  // and store it in a64[0]
        sas     OS_ASSET;  // check sum of assets assignments in output equals a64[0]
        test;

        // Check that sum of inflation rights = max supply - issued supply
        put     a8[0],ERRNO_INFLATION_MISMATCH;  // set errno
        ldg     GS_MAX_SUPPLY,a8[1],s16[1];  // read max supply global state
        extr    s16[1],a64[1],a16[0];  // and store it in a64[1]
        sub.uc  a64[1],a64[0];  // issued supply is still in a64[0], result overwrites a64[0]
        test;  // fails if result is <0
        sas     OS_INFLATION;  // check sum of inflation rights in output equals a64[0]
        test;

        ret;
    };
    Lib::assemble::<Instr<RgbIsa<FilteredContractState>>>(&code)
        .expect("wrong inflatable asset genesis valdiation script")
}

pub(crate) fn ifa_lib_transfer() -> Lib {
    let code = rgbasm! {
        // Checking that the sum of inputs is equal to the sum of outputs
        put     a8[0],ERRNO_NON_EQUAL_IN_OUT;  // set errno
        svs     OS_ASSET;  // verify sum
        test;  // check it didn't fail
        svs     OS_INFLATION;  // verify sum
        test;  // check it didn't fail

        // Link rights validation
        put     a8[0],ERRNO_NON_EQUAL_IN_OUT;  // set errno
        cnp     OS_LINK,a16[0];  // count input link rights
        cns     OS_LINK,a16[1];  // count output link rights
        eq.n    a16[0],a16[1];  // check if input_count == output_count
        test;  // fail if output_count != input_count

        ret;  // return execution flow
    };
    Lib::assemble::<Instr<RgbIsa<FilteredContractState>>>(&code)
        .expect("wrong transfer validation script")
}

pub(crate) fn ifa_lib_inflation() -> Lib {
    #[allow(clippy::diverging_sub_expression)]
    let code = rgbasm! {
        // Set common offsets
        put     a8[1],0;
        put     a16[0],0;

        // Check reported issued supply equals sum of asset allocations in output
        put     a8[0],ERRNO_ISSUED_MISMATCH;  // set errno
        ldg     GS_ISSUED_SUPPLY,a8[1],s16[0];  // read issued supply global state
        extr    s16[0],a64[0],a16[0];  // and store it in a64[0]
        sas     OS_ASSET;  // check sum of asset allocations in output equals issued_supply
        test;
        cpy     a64[0],a64[1];  // store issued supply in a64[1] for later

        // Check reported allowed inflation equals sum of inflation rights in output
        put     a8[0],ERRNO_INFLATION_MISMATCH;  // set errno
        ldm     MS_ALLOWED_INFLATION,s16[0];  // read allowed inflation metadata
        extr    s16[0],a64[0],a16[0];  // and store it in a64[0]
        sas     OS_INFLATION;  // check sum of inflation rights in output equals a64[0]
        test;

        // Check that input inflation rights equals issued supply + allowed inflation
        put     a8[0],ERRNO_INFLATION_EXCEEDS_ALLOWANCE;
        add.uc  a64[1],a64[0];  // result is stored in a64[0]
        test;  // fails in case of an overflow
        sps     OS_INFLATION;  // check sum of inflation rights in input equals a64[0]
        test;

        ret;
    };
    Lib::assemble::<Instr<RgbIsa<FilteredContractState>>>(&code)
        .expect("wrong inflation validation script")
}

pub(crate) fn ifa_lib_burn() -> Lib {
    const BEFORE_LOOP: u16 = 16;
    const LOOP: u16 = 20;
    const LOAD_GLOBAL: u16 = 34;
    const FINAL_CHECKS: u16 = 24;

    const LOOP_STRT_1: u16 = BEFORE_LOOP;
    const LOOP_EXIT_1: u16 = LOOP_STRT_1 + LOOP;
    const LDGL_EXIT_1: u16 = LOOP_EXIT_1 + LOAD_GLOBAL;
    const LOOP_STRT_2: u16 = LDGL_EXIT_1 + FINAL_CHECKS + BEFORE_LOOP;
    const LOOP_EXIT_2: u16 = LOOP_STRT_2 + LOOP;
    const LDGL_EXIT_2: u16 = LOOP_EXIT_2 + LOAD_GLOBAL;

    #[allow(clippy::diverging_sub_expression)]
    let code: Vec<Instr<RgbIsa<FilteredContractState>>> = rgbasm! {
        // 1. VALIDATE OS_ASSET BURN
        // 1.1 BEFORE_LOOP: load sum of OS_ASSET outputs into a64[0]
        put     a8[0],ERRNO_BURN_MISMATCH;  // set errno
        put     a16[0],0; // index
        put     a64[0],0; // accumulator
        cns     OS_ASSET,a16[1]; // count OS_ASSET assignments

        // 1.2 LOOP: loop through all OS_ASSET assignments and sum their values into a64[0]
        // *LOOP_STRT_1* jump destination
        eq.n    a16[0],a16[1];
        jif     LOOP_EXIT_1; // if we cycled all assignments, end loop
        ldf     OS_ASSET,a16[0],a64[1]; // load assignment at the current index
        add.uc  a64[1],a64[0]; // add it to the accumulator
        test;
        inc     a16[0]; // increment index
        jmp     LOOP_STRT_1; // go to the next iteration
        // *LOOP_EXIT_1* jump destination

        // 1.3 LOAD_GLOBAL: load the burned amount into a64[1], defaulting to 0 when absent.
        // GS_BURNED_ASSET is optional since the transition can burn other assignment types
        put     a64[1],0;
        cng     GS_BURNED_ASSET,a8[1];  // check if there is a GS_BURNED_ASSET entry
        ifz     a8[1];
        jif     LDGL_EXIT_1;  // absent -> burned amount is 0

        inv     st0; // restore st0 after ifz check
        put     a8[1],0;  // index of the global state item to read
        ldg     GS_BURNED_ASSET,a8[1],s16[0];
        put     a16[0],0;

        extr    s16[0],a64[1],a16[0];
        test;
        ifz     a64[1]; // must be nonzero if present
        inv     st0;
        test;   // fail if GS_BURNED_ASSET is present and equal to 0
        // *LDGL_EXIT_1* jump destination

        // 1.4 FINAL_CHECKS:
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

        // 2. VALIDATE OS_INFLATION BURN
        // 2.1 BEFORE_LOOP: load sum of OS_INFLATION outputs into a64[0]
        put     a8[0],ERRNO_BURN_MISMATCH;  // set errno
        put     a16[0],0; // index
        put     a64[0],0; // accumulator
        cns     OS_INFLATION,a16[1]; // count OS_INFLATION assignments
        // 2.2 LOOP: loop through all OS_INFLATION assignments and sum their values into a64[0]
        // *LOOP_START_2* jump destination
        eq.n    a16[0],a16[1];
        jif     LOOP_EXIT_2; // if we cycled all assignments, end loop
        ldf     OS_INFLATION,a16[0],a64[1]; // load assignment at the current index
        add.uc  a64[1],a64[0]; // add it to the accumulator
        test;
        inc     a16[0]; // increment index
        jmp     LOOP_STRT_2; // go to the next iteration
        // *LOOP_EXIT_2* jump destination

        // 2.3 LOAD_GLOBAL: load the burned amount into a64[1], defaulting to 0 when absent.
        // GS_BURNED_INFLATION is optional since the transition can burn other assignment types
        put     a64[1],0;
        cng     GS_BURNED_INFLATION,a8[1];  // check if there is a GS_BURNED_INFLATION entry
        ifz     a8[1];
        jif     LDGL_EXIT_2;  // absent -> burned amount is 0

        inv     st0; // restore st0 after ifz check
        put     a8[1],0;  // index of the global state item to read
        ldg     GS_BURNED_INFLATION,a8[1],s16[0];
        put     a16[0],0;

        extr    s16[0],a64[1],a16[0];
        test;
        ifz     a64[1]; // must be nonzero if present
        inv     st0;
        test;   // fail if GS_BURNED_INFLATION is present and equal to 0
        // *LDGL_EXIT_2* jump destination

        // 2.4 FINAL_CHECKS:
        // - sum of inputs must equal burned amount plus sum of outputs
        add.uc  a64[1],a64[0];
        test;
        sps     OS_INFLATION;
        test;
        // - a nonzero amount must be burned (pure transfer is forbidden)
        put     a8[0],ERRNO_BURN_ZERO;  // set errno
        ifz     a64[1];
        st.s    a8[1]; // burned_amount == 0
        ifz     a64[0]; // os_inflation input == 0
        inv     st0;
        st.n    a8[1]; // burned_amount == 0 AND os_inflation input > 0
        ifz     a8[1];
        test; // fail if burned_amount == 0 AND os_inflation input > 0

        ret;
    };

    let aluvm_script = Lib::assemble::<Instr<RgbIsa<FilteredContractState>>>(&code)
        .expect("wrong burn validation script");
    let script_length = aluvm_script.code_segment().len() as u16;
    assert_eq!(script_length, LDGL_EXIT_2 + FINAL_CHECKS + 1 /* ret */);
    aluvm_script
}

fn ifa_standard_types() -> StandardTypes {
    StandardTypes::new([rgb_contract_id_stl(), rgb_burn_stl()])
}

fn ifa_schema() -> Schema {
    let types = ifa_standard_types();

    let alu_id_transfer = ifa_lib_transfer().id();

    Schema {
        ffv: zero!(),
        name: tn!("InflatableFungibleAsset"),
        meta_types: tiny_bmap! {
            MS_ALLOWED_INFLATION => MetaDetails {
                sem_id: types.get("RGBContract.Amount"),
                name: fname!("allowedInflation"),
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
            GS_MAX_SUPPLY => GlobalDetails {
                global_state_schema: GlobalStateSchema::once(types.get("RGBContract.Amount")),
                name: fname!("maxSupply"),
            },
            GS_BURNED_ASSET => GlobalDetails {
                // `many`: one entry accumulates per burn transition, like issuedSupply
                global_state_schema: GlobalStateSchema::many(types.get("RGBContract.Amount")),
                name: fname!("burnedAsset"),
            },
            GS_BURNED_INFLATION => GlobalDetails {
                global_state_schema: GlobalStateSchema::many(types.get("RGBContract.Amount")),
                name: fname!("burnedInflation"),
            },
            GS_BURN_REASON => GlobalDetails {
                global_state_schema: GlobalStateSchema::many(types.get("RGBBurn.BurnReason")),
                name: fname!("burnReason"),
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
        },
        owned_types: tiny_bmap! {
            OS_ASSET => AssignmentDetails {
                owned_state_schema: OwnedStateSchema::Fungible(FungibleType::Unsigned64Bit),
                name: fname!("assetOwner"),
                default_transition: TS_TRANSFER,
            },
            OS_INFLATION => AssignmentDetails {
                owned_state_schema: OwnedStateSchema::Fungible(FungibleType::Unsigned64Bit),
                name: fname!("inflationAllowance"),
                default_transition: TS_TRANSFER
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
                GS_ISSUED_SUPPLY => Occurrences::Once,
                GS_MAX_SUPPLY => Occurrences::Once,
                GS_REJECT_LIST_URL => Occurrences::NoneOrOnce,
                GS_LINKED_FROM_CONTRACT => Occurrences::NoneOrOnce,
            },
            assignments: tiny_bmap! {
                OS_ASSET => Occurrences::NoneOrMore,
                OS_INFLATION => Occurrences::NoneOrMore,
                OS_LINK => Occurrences::NoneOrOnce,
            },
            validator: Some(LibSite::with(0, ifa_lib_genesis().id())),
        },
        transitions: tiny_bmap! {
            TS_TRANSFER => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: none!(),
                    globals: none!(),
                    inputs: tiny_bmap! {
                        OS_ASSET => Occurrences::NoneOrMore,
                        OS_INFLATION => Occurrences::NoneOrMore,
                        OS_LINK => Occurrences::NoneOrOnce,
                    },
                    assignments: tiny_bmap! {
                        OS_ASSET => Occurrences::NoneOrMore,
                        OS_INFLATION => Occurrences::NoneOrMore,
                        OS_LINK => Occurrences::NoneOrOnce,
                    },
                    validator: Some(LibSite::with(0, alu_id_transfer))
                },
                name: fname!("transfer"),
            },
            TS_INFLATION => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: tiny_bset![MS_ALLOWED_INFLATION],
                    globals: tiny_bmap! {
                        GS_ISSUED_SUPPLY => Occurrences::Once,
                    },
                    inputs: tiny_bmap! {
                        OS_INFLATION => Occurrences::OnceOrMore
                    },
                    assignments: tiny_bmap! {
                        OS_ASSET => Occurrences::OnceOrMore,
                        OS_INFLATION => Occurrences::NoneOrMore
                    },
                    validator: Some(LibSite::with(0, ifa_lib_inflation().id()))
                },
                name: fname!("inflate"),
            },
            TS_BURN => TransitionDetails {
                transition_schema: TransitionSchema {
                    metadata: none!(),
                    globals: tiny_bmap! {
                        GS_BURNED_ASSET => Occurrences::NoneOrOnce,
                        GS_BURNED_INFLATION => Occurrences::NoneOrOnce,
                        GS_BURN_REASON => Occurrences::NoneOrOnce,
                    },
                    inputs: tiny_bmap! {
                        OS_ASSET => Occurrences::NoneOrMore,
                        OS_INFLATION => Occurrences::NoneOrMore,
                        OS_LINK => Occurrences::NoneOrOnce,
                    },
                    assignments: tiny_bmap! {
                        OS_ASSET => Occurrences::NoneOrMore,
                        OS_INFLATION => Occurrences::NoneOrMore,
                    },
                    validator: Some(LibSite::with(0, ifa_lib_burn().id()))
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
                    validator: None
                },
                name: fname!("link"),
            },
        },
        default_assignment: Some(OS_ASSET),
    }
}

#[derive(Default)]
pub struct InflatableFungibleAsset;

impl IssuerWrapper for InflatableFungibleAsset {
    type Wrapper<S: ContractStateRead> = IfaWrapper<S>;

    fn schema() -> Schema { ifa_schema() }

    fn libs() -> TypeLibs { ifa_standard_types().libs() }

    fn scripts() -> Scripts {
        let alu_lib_genesis = ifa_lib_genesis();
        let alu_id_genesis = alu_lib_genesis.id();

        let alu_lib_transfer = ifa_lib_transfer();
        let alu_id_transfer = alu_lib_transfer.id();

        let alu_lib_inflation = ifa_lib_inflation();
        let alu_id_inflation = alu_lib_inflation.id();

        let alu_lib_burn = ifa_lib_burn();
        let alu_id_burn = alu_lib_burn.id();

        Confined::from_checked(bmap! {
            alu_id_genesis => alu_lib_genesis,
            alu_id_transfer => alu_lib_transfer,
            alu_id_inflation => alu_lib_inflation,
            alu_id_burn => alu_lib_burn,
        })
    }
}

impl LinkableIssuerWrapper for InflatableFungibleAsset {
    type Wrapper<S: ContractStateRead> = IfaWrapper<S>;
}

#[derive(Clone, Eq, PartialEq, Debug, From)]
pub struct IfaWrapper<S: ContractStateRead>(ContractData<S>);

impl<S: ContractStateRead> SchemaWrapper<S> for IfaWrapper<S> {
    fn with(data: ContractData<S>) -> Self {
        if data.schema().schema_id() != IFA_SCHEMA_ID {
            panic!("the provided schema is not IFA");
        }
        Self(data)
    }
}

impl<S: ContractStateRead> IfaWrapper<S> {
    pub fn spec(&self) -> AssetSpec {
        let strict_val = &self
            .0
            .global("spec")
            .next()
            .expect("IFA requires global state `spec` to have at least one item");
        AssetSpec::from_strict_val_unchecked(strict_val)
    }

    pub fn contract_terms(&self) -> ContractTerms {
        let strict_val = &self
            .0
            .global("terms")
            .next()
            .expect("IFA requires global state `terms` to have at least one item");
        ContractTerms::from_strict_val_unchecked(strict_val)
    }

    pub fn reject_list_url(&self) -> Option<RejectListUrl> {
        self.0
            .global("rejectListUrl")
            .next()
            .map(|strict_val| RejectListUrl::from_strict_val_unchecked(&strict_val))
    }

    fn issued_supply(&self) -> impl Iterator<Item = Amount> + '_ {
        self.0
            .global("issuedSupply")
            .map(|amount| Amount::from_strict_val_unchecked(&amount))
    }

    pub fn total_issued_supply(&self) -> Amount { self.issued_supply().sum() }

    pub fn issuance_amounts(&self) -> Vec<Amount> { self.issued_supply().collect::<Vec<_>>() }

    fn burned(&self, name: &'static str) -> impl Iterator<Item = Amount> + '_ {
        self.0
            .global(name)
            .map(|amount| Amount::from_strict_val_unchecked(&amount))
    }

    /// Amounts burned by each burn transition, in contract state order.
    pub fn burn_amounts(&self, assignment_type: &AssignmentType) -> Vec<Amount> {
        self.burned(burn_global_by_assignment(assignment_type))
            .collect()
    }

    /// Total amount of `assignment_type` burned over the contract's whole history.
    pub fn total_burned(&self, assignment_type: &AssignmentType) -> Amount {
        self.burned(burn_global_by_assignment(assignment_type))
            .sum()
    }

    /// Reasons committed to by burn transitions, in contract state order.
    pub fn burn_reasons(&self) -> impl Iterator<Item = BurnReason> + '_ {
        self.0
            .global("burnReason")
            .map(|strict_val| BurnReason::from_strict_val_unchecked(&strict_val))
    }

    pub fn max_supply(&self) -> Amount {
        self.0
            .global("maxSupply")
            .map(|amount| Amount::from_strict_val_unchecked(&amount))
            .sum()
    }

    pub fn allocations<'c>(
        &'c self,
        filter: impl AssignmentsFilter + 'c,
    ) -> impl Iterator<Item = Result<FungibleAllocation, ContractError>> + 'c {
        self.0.fungible_raw(OS_ASSET, filter)
    }

    pub fn inflation_allocations<'c>(
        &'c self,
        filter: impl AssignmentsFilter + 'c,
    ) -> impl Iterator<Item = Result<FungibleAllocation, ContractError>> + 'c {
        self.0.fungible_raw(OS_INFLATION, filter)
    }
}

impl<S: ContractStateRead> LinkableSchemaWrapper<S> for IfaWrapper<S> {
    fn link_to(&self) -> Result<Option<ContractId>, LinkError> {
        crate::extract_single_contract_id(self.0.global("linkedToContract"))
    }

    fn link_from(&self) -> Result<Option<ContractId>, LinkError> {
        crate::extract_single_contract_id(self.0.global("linkedFromContract"))
    }
}

#[cfg(test)]
mod test {
    use crate::ifa::ifa_schema;
    use crate::IFA_SCHEMA_ID;

    #[test]
    fn schema_id() {
        let schema_id = ifa_schema().schema_id();
        eprintln!("{:#04x?}", schema_id.to_byte_array());
        assert_eq!(IFA_SCHEMA_ID, schema_id);
    }
}
