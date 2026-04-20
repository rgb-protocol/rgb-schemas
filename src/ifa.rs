// RGB schemas
//
// SPDX-License-Identifier: Apache-2.0
//
// Written in 2025 by
//     Stefano Pellegrini <stefano.pellegrini@bitfinex.com>
//
// Copyright (C) 2025 LNP/BP Standards Association. All rights reserved.
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
    AssignmentsFilter, ContractData, FungibleAllocation, IssuerWrapper, LinkError,
    LinkableIssuerWrapper, LinkableSchemaWrapper, SchemaWrapper,
};
use rgbstd::persistence::{ContractStateRead, MemContract};
use rgbstd::rgbcore::stl::rgb_contract_id_stl;
use rgbstd::schema::{
    AssignmentDetails, FungibleType, GenesisSchema, GlobalStateSchema, Occurrences,
    OwnedStateSchema, Schema, TransitionSchema,
};
use rgbstd::stl::{AssetSpec, ContractTerms, RejectListUrl, StandardTypes};
use rgbstd::validation::Scripts;
use rgbstd::vm::RgbIsa;
use rgbstd::{
    rgbasm, Amount, AssignmentType, ContractId, GlobalDetails, MetaDetails, MetaType, SchemaId,
    TransitionDetails,
};
use strict_types::{StrictVal, TypeSystem};

use crate::{
    ERRNO_BURN_MISMATCH, ERRNO_BURN_ZERO, ERRNO_INFLATION_EXCEEDS_ALLOWANCE,
    ERRNO_INFLATION_MISMATCH, ERRNO_ISSUED_MISMATCH, ERRNO_NON_EQUAL_IN_OUT, GS_ISSUED_SUPPLY,
    GS_LINKED_FROM_CONTRACT, GS_LINKED_TO_CONTRACT, GS_MAX_SUPPLY, GS_NOMINAL, GS_REJECT_LIST_URL,
    GS_TERMS, MS_ALLOWED_INFLATION, MS_BURNED_ASSET, MS_BURNED_INFLATION, OS_ASSET, OS_INFLATION,
    OS_LINK, TS_BURN, TS_INFLATION, TS_LINK, TS_TRANSFER,
};

pub const IFA_SCHEMA_ID: SchemaId = SchemaId::from_array([
    0x22, 0x98, 0xc9, 0x84, 0x52, 0xf3, 0xde, 0x8c, 0xb0, 0x60, 0xa4, 0x31, 0x3b, 0x72, 0xa6, 0x16,
    0x04, 0x74, 0x01, 0xae, 0x35, 0xe6, 0x79, 0x53, 0x36, 0xb3, 0x72, 0x10, 0x5a, 0x8c, 0x64, 0x21,
]);

pub fn burn_meta_by_assignment(assignment_type: &AssignmentType) -> MetaType {
    match assignment_type {
        &OS_ASSET => MS_BURNED_ASSET,
        &OS_INFLATION => MS_BURNED_INFLATION,
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
    Lib::assemble::<Instr<RgbIsa<MemContract>>>(&code)
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
    Lib::assemble::<Instr<RgbIsa<MemContract>>>(&code).expect("wrong transfer validation script")
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
    Lib::assemble::<Instr<RgbIsa<MemContract>>>(&code).expect("wrong inflation validation script")
}

pub(crate) fn ifa_lib_burn() -> Lib {
    const BEFORE_LOOP: u16 = 16;
    const LOOP: u16 = 20;
    const AFTER_LOOP: u16 = 36;

    const LOOP_STRT_1: u16 = BEFORE_LOOP;
    const LOOP_EXIT_1: u16 = LOOP_STRT_1 + LOOP;
    const LOOP_STRT_2: u16 = LOOP_EXIT_1 + AFTER_LOOP + BEFORE_LOOP;
    const LOOP_EXIT_2: u16 = LOOP_STRT_2 + LOOP;

    #[allow(clippy::diverging_sub_expression)]
    let code: Vec<Instr<RgbIsa<MemContract>>> = rgbasm! {
        // 1. VALIDATE OS_ASSET BURN
        // 1.1 load sum of OS_ASSET outputs into a64[0]
        put     a8[0],ERRNO_BURN_MISMATCH;  // set errno
        put     a16[0],0; // index
        put     a64[0],0; // accumulator
        cns     OS_ASSET,a16[1]; // count OS_ASSET assignments
        // loop through all OS_ASSET assignments and sum their values into a64[0]
        // *LOOP_START_1* jump destination
        eq.n    a16[0],a16[1];
        jif     LOOP_EXIT_1; // if we cycled all assignments, end loop
        ldf     OS_ASSET,a16[0],a64[1]; // load assignment at the current index
        add.uc  a64[1],a64[0]; // add it to the accumulator
        test;
        inc     a16[0]; // increment index
        jmp     LOOP_STRT_1; // go to the next iteration
        // *LOOP_EXIT_1* jump destination

        // 1.2 check that sum of inputs equals burned amount plus sum of outputs
        put     a16[0],0;
        ldm     MS_BURNED_ASSET,s16[0];
        extr    s16[0],a64[1],a16[0];
        test;
        add.uc  a64[1],a64[0];
        test;
        sps     OS_ASSET;
        test;

        // 1.3 check that some amount is actually burned (pure transfer is forbidden)
        // fail if MS_BURNED_ASSET = 0 AND there are OS_ASSET inputs
        put     a8[0],ERRNO_BURN_ZERO;  // set errno
        ifz     a64[1];
        st.s    a8[1]; // burned_amount metadata = 0
        ifz     a64[0]; // we already checked this equals the sum of inputs
        inv     st0;
        st.n    a8[1]; // AND os_asset input > 0
        ifz     a8[1];
        test;

        // 2. VALIDATE OS_INFLATION BURN
        // 2.1 load sum of OS_INFLATION outputs into a64[0]
        put     a8[0],ERRNO_BURN_MISMATCH;  // set errno
        put     a16[0],0; // index
        put     a64[0],0; // accumulator
        cns     OS_INFLATION,a16[1]; // count OS_INFLATION assignments
        // loop through all OS_INFLATION assignments and sum their values into a64[0]
        // *LOOP_START_2* jump destination
        eq.n    a16[0],a16[1];
        jif     LOOP_EXIT_2; // if we cycled all assignments, end loop
        ldf     OS_INFLATION,a16[0],a64[1]; // load assignment at the current index
        add.uc  a64[1],a64[0]; // add it to the accumulator
        test;
        inc     a16[0]; // increment index
        jmp     LOOP_STRT_2; // go to the next iteration
        // *LOOP_EXIT_2* jump destination

        // 2.2 check that sum of inputs equals burned amount plus sum of outputs
        put     a16[0],0;
        ldm     MS_BURNED_INFLATION,s16[0];
        extr    s16[0],a64[1],a16[0];
        test;
        add.uc  a64[1],a64[0];
        test;
        sps     OS_INFLATION;
        test;

        // 2.3 check that some amount is actually burned (pure transfer is forbidden)
        // fail if MS_BURNED_INFLATION = 0 AND there are OS_INFLATION inputs
        put     a8[0],ERRNO_BURN_ZERO;  // set errno
        ifz     a64[1];
        st.s    a8[1]; // burned_amount metadata = 0
        ifz     a64[0]; // we already checked this equals the sum of inputs
        inv     st0;
        st.n    a8[1]; // AND os_inflation input > 0
        ifz     a8[1];
        test;

        ret;
    };
    let aluvm_script =
        Lib::assemble::<Instr<RgbIsa<MemContract>>>(&code).expect("wrong burn validation script");
    let script_length = aluvm_script.code_segment().len() as u16;
    assert_eq!(script_length, LOOP_EXIT_2 + AFTER_LOOP + 1 /* ret */);
    aluvm_script
}

fn ifa_standard_types() -> StandardTypes { StandardTypes::with(rgb_contract_id_stl()) }

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
            burn_meta_by_assignment(&OS_ASSET) => MetaDetails {
                sem_id: types.get("RGBContract.Amount"),
                name: fname!("burnedAsset"),
            },
            burn_meta_by_assignment(&OS_INFLATION) => MetaDetails {
                sem_id: types.get("RGBContract.Amount"),
                name: fname!("burnedInflation"),
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
                    metadata: Confined::from_iter_checked([OS_ASSET, OS_INFLATION].iter().map(burn_meta_by_assignment)),
                    globals: none!(),
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

    fn types() -> TypeSystem { ifa_standard_types().type_system(ifa_schema()) }

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
        if data.schema.schema_id() != IFA_SCHEMA_ID {
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

    pub fn max_supply(&self) -> Amount {
        self.0
            .global("maxSupply")
            .map(|amount| Amount::from_strict_val_unchecked(&amount))
            .sum()
    }

    pub fn allocations<'c>(
        &'c self,
        filter: impl AssignmentsFilter + 'c,
    ) -> impl Iterator<Item = FungibleAllocation> + 'c {
        self.0.fungible_raw(OS_ASSET, filter).unwrap()
    }

    pub fn inflation_allocations<'c>(
        &'c self,
        filter: impl AssignmentsFilter + 'c,
    ) -> impl Iterator<Item = FungibleAllocation> + 'c {
        self.0.fungible_raw(OS_INFLATION, filter).unwrap()
    }
}

fn extract_global_single_val(
    mut global: impl Iterator<Item = StrictVal>,
) -> Result<Option<ContractId>, LinkError> {
    let Some(val) = global.next() else {
        return Ok(None);
    };
    if global.next().is_some() {
        return Err(LinkError::MultipleValues);
    }
    Ok(Some(ContractId::from_strict_val_unchecked(val)))
}

impl<S: ContractStateRead> LinkableSchemaWrapper<S> for IfaWrapper<S> {
    fn link_to(&self) -> Result<Option<ContractId>, LinkError> {
        extract_global_single_val(self.0.global("linkedToContract"))
    }

    fn link_from(&self) -> Result<Option<ContractId>, LinkError> {
        extract_global_single_val(self.0.global("linkedFromContract"))
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
