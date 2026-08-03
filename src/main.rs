// RGB schemas
//
// SPDX-License-Identifier: Apache-2.0
//
// Written in 2023-2024 by
//     Dr Maxim Orlovsky <orlovsky@lnp-bp.org>
//
// Copyright (C) 2023-2024 LNP/BP Standards Association. All rights reserved.
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

use std::io;
use std::io::stdout;

use rgbstd::containers::FileContent;
use rgbstd::contract::IssuerWrapper;
use rgbstd::persistence::MemContract;
use rgbstd::validation::SchemaDefinition;
use rgbstd::vm::RgbIsa;
use schemata::{
    CollectibleFungibleAsset, InflatableFungibleAsset, NonInflatableAsset,
    PermissionedFungibleAsset, UniqueDigitalAsset,
};

fn main() -> io::Result<()> {
    cfa()?;
    ifa()?;
    nia()?;
    pfa()?;
    uda()?;

    Ok(())
}

fn nia() -> io::Result<()> {
    let schema_def = NonInflatableAsset::schema_definition();

    schema_def.save_file("schemata/NonInflatableAsset.rgb")?;
    schema_def.save_armored("schemata/NonInflatableAsset.rgba")?;
    print_lib(&schema_def);

    Ok(())
}

fn pfa() -> io::Result<()> {
    let schema_def = PermissionedFungibleAsset::schema_definition();

    schema_def.save_file("schemata/PermissionedFungibleAsset.rgb")?;
    schema_def.save_armored("schemata/PermissionedFungibleAsset.rgba")?;
    print_lib(&schema_def);

    Ok(())
}

fn uda() -> io::Result<()> {
    let schema_def = UniqueDigitalAsset::schema_definition();

    schema_def.save_file("schemata/UniqueDigitalAsset.rgb")?;
    schema_def.save_armored("schemata/UniqueDigitalAsset.rgba")?;
    print_lib(&schema_def);

    Ok(())
}

fn cfa() -> io::Result<()> {
    let schema_def = CollectibleFungibleAsset::schema_definition();

    schema_def.save_file("schemata/CollectibleFungibleAsset.rgb")?;
    schema_def.save_armored("schemata/CollectibleFungibleAsset.rgba")?;
    print_lib(&schema_def);

    Ok(())
}

fn ifa() -> io::Result<()> {
    let schema_def = InflatableFungibleAsset::schema_definition();

    schema_def.save_file("schemata/InflatableFungibleAsset.rgb")?;
    schema_def.save_armored("schemata/InflatableFungibleAsset.rgba")?;
    print_lib(&schema_def);

    Ok(())
}

fn print_lib(schema_def: &SchemaDefinition) {
    let alu_lib = schema_def.scripts.values().next().unwrap();
    eprintln!("{alu_lib}");
    alu_lib
        .print_disassemble::<RgbIsa<MemContract>>(stdout())
        .unwrap();
}
