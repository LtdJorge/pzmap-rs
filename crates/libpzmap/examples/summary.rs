//! Prints a summary of one cell and, optionally, of a texture pack.
//!
//! ```text
//! cargo run --example summary -- <map-dir> <x> <y> [pack-file]
//! ```
//!
//! The Java and Python bindings have the same example, so you can compare their output with
//! this one's.

use libpzmap::{
    Build,
    lotheader::AnyLotHeader,
    lotpack::AnyCell,
    texturepack::TexturePack,
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);

    let dir = PathBuf::from(
        args.next()
            .ok_or("usage: summary <map-dir> <x> <y> [pack]")?,
    );
    let x: i32 = args.next().ok_or("missing x")?.parse()?;
    let y: i32 = args.next().ok_or("missing y")?.parse()?;
    let pack = args.next();

    // Change this to Build::B41 to read a Build 41 map.
    let build = Build::B42;

    let header_bytes = std::fs::read(dir.join(format!("{x}_{y}.lotheader")))?;
    let header = AnyLotHeader::parse(build, x, y, header_bytes)?;

    println!("build          {build}");
    println!("cell           {x},{y}");
    println!("strings        {}", header.strings().len());
    println!("rooms          {}", header.rooms().len());
    println!("buildings      {}", header.buildings().len());
    println!(
        "layers         {}..{}",
        header.min_layer(),
        header.max_layer()
    );
    println!("trailing bytes {}", header.trailing_bytes());

    let pack_bytes = std::fs::read(dir.join(format!("world_{x}_{y}.lotpack")))?;
    let cell = AnyCell::parse(
        build,
        x,
        y,
        &pack_bytes,
        header.min_layer(),
        header.max_layer(),
    )?;

    println!("squares        {}", cell.len());
    println!("tiles          {}", cell.tiles().len());

    if let Some(pack) = pack {
        let bytes = std::fs::read(&pack)?;
        let atlas = TexturePack::parse(&bytes)?;
        println!("pages          {}", atlas.pages.len());
        println!("textures       {}", atlas.texture_count());
    }

    Ok(())
}
