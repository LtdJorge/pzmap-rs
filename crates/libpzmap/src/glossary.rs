//! The terms used in this crate to describe the parts of a Project Zomboid map.
//!
//! From largest to smallest, a map is made of cells, a cell of blocks, and a block of squares.
//! Each square holds tiles, and each tile is a sprite coming from a texture pack.
//!
//! ```text
//! map
//!  `-- cell           256 x 256 squares, stored as a lotheader and a lotpack
//!       `-- block     8 x 8 squares, on every layer
//!            `-- square
//!                 `-- tiles, drawn bottom to top
//!                      `-- each tile's image is a sprite in a texture pack
//! ```
//!
//! The sizes are for Build 42. The sections below give the Build 41 sizes.
//!
//! # Cell
//!
//! A square area of the map, 256×256 squares in Build 42 or 300×300 in Build 41.
//!
//! The world is a grid of cells. Each cell is stored as a [lotheader](#lotheader) and a
//! [lotpack](#lotpack), both named after the cell's position in that grid: `world_49_6.lotpack`
//! is cell (49, 6). Multiply the cell position by the cell size to get the world position of its
//! first square.
//!
//! ```text
//!              x ->
//!               48           49           50
//!           +------------+------------+------------+
//!   y    5  |            |            |            |
//!   |       +------------+------------+------------+
//!   v    6  |            | cell 49, 6 |            |
//!           |            |            |            |
//!           +------------+------------+------------+
//!
//!   cell (49, 6) in Build 42:
//!     files         49_6.lotheader and world_49_6.lotpack
//!     first square  (49 * 256, 6 * 256) = (12544, 1536)
//! ```
//!
//! # Block
//!
//! A group of [squares](#square) that the game loads and saves together.
//!
//! A block is 8×8 squares in Build 42 and 10×10 in Build 41, so a cell is 32×32 or 30×30 blocks.
//! A block covers that area on every [layer](#layer). The [lotpack](#lotpack) stores each block
//! separately and lists the byte offset to each at the start of the file.
//!
//! ```text
//!   one cell in Build 42: 32 x 32 blocks       one block: 8 x 8 squares
//!
//!     block_x ->                                  x ->
//!     +----+----+----+-- --+-----+                +--+--+--+--+--+--+--+--+
//!     |  0 | 32 | 64 |     | 992 |              y | 0| 1| 2| 3| 4| 5| 6| 7|
//!     +----+----+----+     +-----+              | +--+--+--+--+--+--+--+--+
//!     |  1 | 33 |                               v | 1|  |  |  |  |  |  |  |
//!     +----+----+                                 +--+--+--+--+--+--+--+--+
//!     |  2 |                                      : 7                     :
//!     :    :                    :                 +--+--+--+--+--+--+--+--+
//!     +----+                 +-----+
//!     | 31 |                 |1023 |
//!     +----+                 +-----+
//!
//!   block number = block_x * 32 + block_y
//! ```
//!
//! The numbers count down each column first. The lotpack's offset table and the lotheader's
//! zombie population use this order.
//!
//! # Square
//!
//! One floor position on one [layer](#layer), the smallest unit of the map.
//!
//! A square holds a list of [tiles](#tile), drawn bottom to top. Most squares are empty,
//! especially above ground, so the [lotpack](#lotpack) stores runs of empty squares as a count
//! (run-length encoding) instead of one entry full of zeroes per square.
//!
//! ```text
//!   squares in file order:   [floor] [    ] [    ] [    ] [floor, wall] [    ] ...
//!   stored as:               tiles   3 empty squares       tiles         ...
//! ```
//!
//! A run of empty squares can continue into the next row of the block and into the next layer.
//!
//! # Layer
//!
//! One level of the map, in the game's terms. Layer 0 is ground level.
//!
//! Build 41 spans layers 0 to 7. Build 42 spans -32 to 31. Negative layers are basements.
//!
//! ```text
//!   side view of a building
//!
//!     layer  2       +-------+
//!     layer  1       |       |
//!     layer  0  _____|_______|_____   ground level
//!     layer -1       |       |        basement
//!                    +-------+
//! ```
//!
//! Each cell stores only the range of layers it uses, which its lotheader records.
//!
//! # Tile
//!
//! One sprite placed on a [square](#square), such as a floor, a wall, or a piece of furniture.
//!
//! A tile has a name such as `vegetation_trees_01_9`. The [lotpack](#lotpack) stores it as a
//! number that indexes the [lotheader](#lotheader)'s list of names, so you need both files to
//! know what is on a square. A [texture pack](#texture-pack) stores the tile's image under the
//! same name. Which means that to get the raw image for a tile in a [lotpack](#lotpack), the
//! tile has to be looked up by index in the [lotheader](#lotheader) to retrieve its name, looked
//! up by that name in the [texture pack's](#texture-pack) texture list to retrieve a texture, and
//! indexing into the PNG page by the positions in the texture to extract the bytes.
//!
//! ```text
//!   lotpack                 lotheader                   texture pack
//!   ---------------         ------------------------    -----------------------------
//!   square (3, 4)           tile names                  page
//!     tile 17  ----------->  17: vegetation_trees_01_9 -->  texture vegetation_trees_01_9
//!                                                             x, y, width, height
//!                                                                  |
//!                                                                  v
//!                                                           that rectangle of
//!                                                           the page's PNG image
//! ```
//!
//! A square's tiles are drawn bottom to top:
//!
//! ```text
//!   one square
//!     3  chair        drawn last, on top
//!     2  wall
//!     1  floor        drawn first
//! ```
//!
//! # Lotheader
//!
//! The `X_Y.lotheader` file of a [cell](#cell).
//!
//! It lists every tile name used by the cell, the cell's [rooms](#room) and
//! [buildings](#building), its range of [layers](#layer), and a zombie population value for each
//! [block](#block). See [`lotheader`](crate::lotheader).
//!
//! ```text
//!   X_Y.lotheader, in file order
//!   +-----------------------------+
//!   | header: LOTH and a version  |  optional
//!   | tile names                  |  one per line, indexed by the lotpack
//!   | block size                  |  8 in Build 42, 10 in Build 41
//!   | layer range                 |  lowest and one past the highest
//!   | rooms                       |
//!   | buildings                   |
//!   | zombie population           |  one byte per block
//!   +-----------------------------+
//! ```
//!
//! # Lotpack
//!
//! The `world_X_Y.lotpack` file of a [cell](#cell).
//!
//! It records which [tiles](#tile) sit on each [square](#square), block by block. It does not
//! record its own layer range, so reading it needs the matching [lotheader](#lotheader). See
//! [`lotpack`](crate::lotpack).
//!
//! ```text
//!   world_X_Y.lotpack, in file order
//!   +--------------------------------+
//!   | header: LOTP and a version     |  optional
//!   | block count                    |  1024 in Build 42, 900 in Build 41
//!   | offset of block 0              |
//!   | offset of block 1              |
//!   | ...                            |
//!   | block 0: squares on each layer |
//!   | block 1: squares on each layer |
//!   | ...                            |
//!   +--------------------------------+
//! ```
//!
//! # Texture pack
//!
//! A `.pack` file holding the images for many [tiles](#tile) in spritesheets.
//!
//! A pack contains a few large PNG images called pages. Each page comes with a table that gives
//! the name of every sprite on it and the rectangle it occupies. See
//! [`texturepack`](crate::texturepack).
//!
//! ```text
//!   Tiles2x.pack
//!   +-------------------------------------------+
//!   | header: PZPK and a version                |  optional
//!   | page count                                |
//!   | page 0                                    |
//!   |   name                                    |
//!   |   textures: name, rectangle, original size|
//!   |   PNG image                               |
//!   | page 1                                    |
//!   | ...                                       |
//!   +-------------------------------------------+
//!
//!   one page's PNG image
//!   +---------------------------------+
//!   | +-----+ +-----+ +---+           |
//!   | |  a  | |  b  | | c |  ...      |   each texture names one
//!   | +-----+ +-----+ +---+           |   rectangle of the image
//!   | +---------+ +-----+             |
//!   | |    d    | |  e  |             |
//!   | +---------+ +-----+             |
//!   +---------------------------------+
//! ```
//!
//! # Room
//!
//! A named area of a [cell](#cell), such as `kitchen`, made of one or more rectangles.
//!
//! The [lotheader](#lotheader) lists the rooms. Each room sits on one [layer](#layer) and may
//! contain [objects](#object).
//!
//! ```text
//!   one layer of a cell
//!
//!     +-----------+-------+
//!     |  kitchen  |       |
//!     |           |       |
//!     +-----+-----+  hall |      the hall is one room
//!     | bed |             |      made of two rectangles
//!     +-----+-------------+
//! ```
//!
//! # Building
//!
//! A set of [rooms](#room) forming one structure.
//!
//! The [lotheader](#lotheader) stores a building as a list of room numbers. The building's shape
//! is the combined shape of its rooms.
//!
//! ```text
//!   building 4 = rooms 12, 13 and 14
//!
//!     +---------+-----+
//!     | room 12 |     |
//!     +---------+ 14  |
//!     | room 13 |     |
//!     +---------+-----+
//! ```
//!
//! # Object
//!
//! A game object placed inside a [room](#room).
//!
//! The [lotheader](#lotheader) gives each one a position and a numeric type. This crate reports
//! the number without interpreting it.
//!
//! # Vehicle
//!
//! A vehicle placed in the world.
//!
//! Vehicles are stored in a save game's `vehicles.db` SQLite file, not in the map files. This
//! crate does not read them yet.
//!
//! # Header
//!
//! The optional marker at the start of a file that says which version wrote it.
//!
//! A lotheader can start with `LOTH`, a lotpack with `LOTP` and a texture pack with `PZPK`, each
//! followed by a version number. Files without the marker are version 0. A header in this sense
//! is a few bytes at the start of a file, not a [lotheader](#lotheader).
//!
//! ```text
//!   with a header                       without a header (version 0)
//!   +------+---------+-----------       +------------------------
//!   | LOTP | version | data ...         | data ...
//!   +------+---------+-----------       +------------------------
//!    4 bytes  4 bytes
//! ```
