//! The native half of the Java bindings for [`libpzmap`].
//!
//! The Java classes in `java/` call the `Java_*` functions in this file through JNI. This crate
//! builds them into a shared library.
//!
//! Most libpzmap types borrow the byte buffer they were parsed from, and a Java object cannot hold
//! a Rust borrow. So this file uses [`AnyLotHeader`], [`AnyCell`] and [`OwnedTexturePack`], which
//! keep their buffer with them. Each parsed value is boxed, and the Java object stores the box's
//! address as a `long` handle. Closing the Java object frees the box.
//!
//! Data crosses into Java in bulk. A cell holds hundreds of thousands of squares and millions of
//! tile indices, and one JNI call per square would take far longer than the parse itself. So the
//! tiles of a whole cell come back as one `int[]`, the square positions as another, and the Java
//! side slices them.
//!
//! Each `Java_*` function matches a `private static native` method in a Java class, and its
//! symbol name spells out that package, class and method. If you rename one side without the
//! other, the code still compiles, and the first call throws `UnsatisfiedLinkError`.

#![allow(non_snake_case)]

use jni::{
    Env,
    EnvUnowned,
    errors::{
        Error as JniError,
        ErrorPolicy,
        Result as JniResult,
    },
    jni_str,
    objects::{
        JByteArray,
        JClass,
        JIntArray,
        JLongArray,
        JObject,
        JObjectArray,
        JString,
        JThrowable,
    },
    signature::RuntimeMethodSignature,
    strings::{
        JNIStr,
        JNIString,
    },
    sys::{
        jint,
        jlong,
    },
};
use libpzmap::{
    Build,
    ParseError,
    lotheader::AnyLotHeader,
    lotpack::AnyCell,
    texturepack::OwnedTexturePack,
};
use std::any::Any;

/// Fully qualified name of the exception thrown for a parse failure, in JNI's slash form.
const EXCEPTION_CLASS: &JNIStr = jni_str!("io/github/ltdjorge/pzmap/PzmapParseException");

/// What went wrong on the Rust side of a native call.
enum Failure {
    /// The file could not be read.
    Io(String),
    /// The file was read but did not parse.
    Parse(ParseError),
    /// A handle or build argument was not one this library handed out.
    Argument(String),
    /// An index argument was outside the collection it indexes.
    Index(String),
    /// A JNI call itself failed. This usually means the JVM is out of memory, or the Java classes
    /// do not match this library's version.
    Jni(JniError),
}

impl From<ParseError> for Failure {
    fn from(error: ParseError) -> Failure {
        Failure::Parse(error)
    }
}

impl From<JniError> for Failure {
    fn from(error: JniError) -> Failure {
        Failure::Jni(error)
    }
}

/// Turns a Rust error or panic into a pending Java exception, and returns a default value.
///
/// A panic must not unwind into the JVM, because that aborts the process, so it is caught here.
/// libpzmap reports malformed input as an error and panics only on its own bugs. A panic therefore
/// becomes a plain `RuntimeException`, which callers are not expected to catch.
struct ThrowFailure;

impl<T: Default> ErrorPolicy<T, Failure> for ThrowFailure {
    type Captures<'unowned: 'method, 'method> = ();

    fn on_error<'unowned: 'method, 'method>(
        env: &mut Env<'unowned>,
        _captures: &mut (),
        error: Failure,
    ) -> JniResult<T> {
        throw(env, &error);
        Ok(T::default())
    }

    fn on_panic<'unowned: 'method, 'method>(
        env: &mut Env<'unowned>,
        _captures: &mut (),
        payload: Box<dyn Any + Send + 'static>,
    ) -> JniResult<T> {
        let message = panic_message(&payload);
        let _ = env.throw_new(
            jni_str!("java/lang/RuntimeException"),
            JNIString::new(format!("libpzmap panicked: {message}")),
        );
        Ok(T::default())
    }
}

/// Returns the message of a panic payload, which in practice is a `&str` or a `String`.
fn panic_message(payload: &Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "panic with a non-string payload".to_owned()
}

/// Throws `failure` as a Java exception.
///
/// A parse failure becomes a `PzmapParseException` that carries the byte offset and the format, so
/// Java code can inspect those fields instead of the message. Other failures become the closest
/// standard Java exception.
fn throw(env: &mut Env<'_>, failure: &Failure) {
    match failure {
        Failure::Io(message) => {
            let _ = env.throw_new(jni_str!("java/io/IOException"), JNIString::new(message));
        }
        Failure::Argument(message) => {
            let _ = env.throw_new(
                jni_str!("java/lang/IllegalArgumentException"),
                JNIString::new(message),
            );
        }
        Failure::Index(message) => {
            let _ = env.throw_new(
                jni_str!("java/lang/IndexOutOfBoundsException"),
                JNIString::new(message),
            );
        }
        Failure::Jni(error) => {
            // If the JNI call failed because Java code threw, that exception is already pending.
            // Replacing it would hide the real cause.
            if !env.exception_check() {
                let _ = env.throw_new(
                    jni_str!("java/lang/RuntimeException"),
                    JNIString::new(format!("JNI error: {error}")),
                );
            }
        }
        Failure::Parse(error) => {
            if let Err(cause) = throw_parse_exception(env, error) {
                // This usually means the jar and the native library come from different versions,
                // so the class is missing or has another constructor. Report that in the message.
                let _ = env.throw_new(
                    jni_str!("java/io/IOException"),
                    JNIString::new(format!(
                        "{error} (could not throw {EXCEPTION_CLASS}: {cause})"
                    )),
                );
            }
        }
    }
}

/// Creates and throws a `PzmapParseException`.
///
/// If the exception cannot be created, this returns the JNI error instead, so the caller can
/// include it in a fallback exception.
fn throw_parse_exception(env: &mut Env<'_>, error: &ParseError) -> Result<(), JniError> {
    let class = env.find_class(EXCEPTION_CLASS)?;
    let message = env.new_string(error.to_string())?;
    let format = env.new_string(error.format.name())?;
    let offset = i64::try_from(error.at).unwrap_or(-1);

    // This must match the constructor `PzmapParseException(String, long, String)`. The jni crate
    // checks the arguments against it, so a mismatch returns an error instead of crashing.
    let signature = RuntimeMethodSignature::from_str("(Ljava/lang/String;JLjava/lang/String;)V")?;
    let exception = env.new_object(
        &class,
        signature.method_signature(),
        &[(&message).into(), offset.into(), (&format).into()],
    )?;

    let throwable = env.cast_local::<JThrowable>(exception)?;

    // `Env::throw` returns `Err(Error::JavaException)` when it succeeds, so that `?` passes the
    // pending exception up in normal code. Here a pending exception is the goal, so that result
    // counts as success.
    match env.throw(throwable) {
        Err(JniError::JavaException) => Ok(()),
        Err(other) => Err(other),
        Ok(()) => Ok(()),
    }
}

/// Runs `f` inside a native method and throws any error or panic as a Java exception.
fn run<'local, R: Default>(
    env: &mut EnvUnowned<'local>,
    f: impl FnOnce(&mut Env<'local>) -> Result<R, Failure>,
) -> R {
    env.with_env(f).resolve::<ThrowFailure>()
}

/// Runs `f` on the value behind a handle and throws any error or panic as a Java exception.
///
/// `T` must be the type that the class's `nativeParse*` function boxed. Each `Java_*` function
/// below belongs to one Java class and names that class's type, so the two always match.
fn with_handle<'local, T, R: Default>(
    env: &mut EnvUnowned<'local>,
    handle: jlong,
    what: &str,
    f: impl FnOnce(&mut Env<'local>, &T) -> Result<R, Failure>,
) -> R {
    run(env, |env| {
        if handle == 0 {
            return Err(Failure::Argument(format!("{what} handle is closed")));
        }
        // SAFETY: Java passes only a pointer from `into_handle::<T>`, and `NativeHandle` holds its
        // read lock for the whole call, so `close` cannot free the pointer until this returns.
        let value = unsafe { (handle as *const T).as_ref() }
            .ok_or_else(|| Failure::Argument(format!("{what} handle is null")))?;
        f(env, value)
    })
}

/// Takes back ownership of a handle and frees the value.
///
/// # Safety
///
/// `handle` must come from `into_handle::<T>`, and must not be used again afterwards.
unsafe fn free<T>(handle: jlong) {
    if handle == 0 {
        return;
    }
    // SAFETY: the caller guarantees the handle came from `Box::into_raw` and is live.
    drop(unsafe { Box::from_raw(handle as *mut T) });
}

/// Moves `value` onto the heap and returns its address as a `jlong` handle.
fn into_handle<T>(value: T) -> jlong {
    Box::into_raw(Box::new(value)) as jlong
}

fn read_file(path: &str) -> Result<Vec<u8>, Failure> {
    std::fs::read(path).map_err(|e| Failure::Io(format!("{path}: {e}")))
}

/// Converts a length or count to a `jint`, the type of a Java array index.
///
/// Nothing this library returns has more than `i32::MAX` elements. The largest collection is a
/// cell's tile list, and a lotpack's own `u32` offsets limit the file to far less than that.
fn count(value: impl TryInto<jint>, what: &str) -> Result<jint, Failure> {
    value
        .try_into()
        .map_err(|_| Failure::Argument(format!("{what} does not fit in an int")))
}

/// Creates a Java `int[]` from a slice.
fn int_array<'local>(env: &mut Env<'local>, values: &[jint]) -> Result<JIntArray<'local>, Failure> {
    let array = env.new_int_array(values.len())?;
    array.set_region(env, 0, values)?;
    Ok(array)
}

/// Creates a Java `long[]` from a slice.
fn long_array<'local>(
    env: &mut Env<'local>,
    values: &[jlong],
) -> Result<JLongArray<'local>, Failure> {
    let array = env.new_long_array(values.len())?;
    array.set_region(env, 0, values)?;
    Ok(array)
}

/// Creates a Java `String[]` of `len` elements from an iterator of strings.
fn string_array<'local, I, S>(
    env: &mut Env<'local>,
    values: I,
    len: usize,
) -> Result<JObjectArray<'local>, Failure>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let class = env.find_class(jni_str!("java/lang/String"))?;
    let array =
        env.new_object_array(count(len, "string array length")?, &class, JObject::null())?;

    for (index, value) in values.into_iter().enumerate() {
        let string = env.new_string(value.as_ref())?;
        array.set_element(env, index, &string)?;
    }

    Ok(array)
}

/// Creates a Java `Object[]` holding `values`, so one call can return several arrays.
fn object_array<'local>(
    env: &mut Env<'local>,
    values: &[&JObject<'local>],
) -> Result<JObjectArray<'local>, Failure> {
    let class = env.find_class(jni_str!("java/lang/Object"))?;
    let array = env.new_object_array(
        count(values.len(), "object array length")?,
        &class,
        JObject::null(),
    )?;
    for (index, value) in values.iter().enumerate() {
        array.set_element(env, index, *value)?;
    }
    Ok(array)
}

/// Flattens `items` into one array, returning the values and where each item's values start.
///
/// `push` appends the values of one item. The starts array is one longer than `items`, so item
/// `i` owns `values[starts[i]..starts[i + 1]]`.
fn runs<T>(
    items: &[T],
    mut push: impl FnMut(&T, &mut Vec<jint>) -> Result<(), Failure>,
) -> Result<(Vec<jint>, Vec<jint>), Failure> {
    let mut values = Vec::new();
    let mut starts = Vec::with_capacity(items.len() + 1);
    starts.push(0);
    for item in items {
        push(item, &mut values)?;
        starts.push(count(values.len(), "run length")?);
    }
    Ok((values, starts))
}

/// Returns the element at a Java index, or an index error if it is out of range.
fn nth<'a, T>(items: &'a [T], index: jint, what: &str) -> Result<&'a T, Failure> {
    usize::try_from(index)
        .ok()
        .and_then(|index| items.get(index))
        .ok_or_else(|| Failure::Index(format!("{what} index {index} out of range")))
}

/// Returns the build for an ordinal of the Java `Build` enum.
fn build_from_ordinal(ordinal: jint) -> Result<Build, Failure> {
    match ordinal {
        0 => Ok(Build::B41),
        1 => Ok(Build::B42),
        other => Err(Failure::Argument(format!("unknown build ordinal {other}"))),
    }
}

// ---------------------------------------------------------------------------------------------
// LotHeader
// ---------------------------------------------------------------------------------------------

/// Parses a `.lotheader` file and returns a handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeParsePath<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
    x: jint,
    y: jint,
    build: jint,
) -> jlong {
    run(&mut env, |env| {
        let path = path.try_to_string(env)?;
        let build = build_from_ordinal(build)?;
        let bytes = read_file(&path)?;
        Ok(into_handle(AnyLotHeader::parse(build, x, y, bytes)?))
    })
}

/// Parses a `.lotheader` from a byte array and returns a handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeParseBytes<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    data: JByteArray<'local>,
    x: jint,
    y: jint,
    build: jint,
) -> jlong {
    run(&mut env, |env| {
        let bytes = env.convert_byte_array(&data)?;
        let build = build_from_ordinal(build)?;
        Ok(into_handle(AnyLotHeader::parse(build, x, y, bytes)?))
    })
}

/// Frees a `.lotheader` handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeFree<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    // SAFETY: Java only ever passes a handle from `nativeParse*`, exactly once.
    unsafe { free::<AnyLotHeader>(handle) };
}

/// Returns the header's scalar fields as `[x, y, version, minLayer, maxLayer, trailingBytes]`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeSummary<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JIntArray<'local> {
    with_handle(
        &mut env,
        handle,
        "LotHeader",
        |env, header: &AnyLotHeader| {
            let values = [
                header.x(),
                header.y(),
                count(header.version(), "version")?,
                header.min_layer(),
                header.max_layer(),
                count(header.trailing_bytes(), "trailing bytes")?,
            ];
            int_array(env, &values)
        },
    )
}

/// Returns every distinct tile name the cell uses.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeStrings<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObjectArray<'local> {
    with_handle(
        &mut env,
        handle,
        "LotHeader",
        |env, header: &AnyLotHeader| {
            let strings = header.strings();
            string_array(env, strings, strings.len())
        },
    )
}

/// Returns the zombie population grid, one byte per block, indexed by `block_x * side + block_y`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeZpop<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JByteArray<'local> {
    with_handle(
        &mut env,
        handle,
        "LotHeader",
        |env, header: &AnyLotHeader| Ok(env.byte_array_from_slice(header.zpop())?),
    )
}

/// Returns the number of blocks along one edge of the zombie population grid.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeZpopSide<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jint {
    with_handle(
        &mut env,
        handle,
        "LotHeader",
        |_env, header: &AnyLotHeader| count(header.zpop_side(), "zpop side"),
    )
}

/// Returns the zombie population of one block, or -1 if either coordinate is outside the grid.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeZpopAt<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    block_x: jint,
    block_y: jint,
) -> jint {
    with_handle(
        &mut env,
        handle,
        "LotHeader",
        |_env, header: &AnyLotHeader| {
            let value = usize::try_from(block_x)
                .ok()
                .zip(usize::try_from(block_y).ok())
                .and_then(|(x, y)| header.zpop_at(x, y));
            Ok(value.map_or(-1, jint::from))
        },
    )
}

/// Returns every room as `[String[] names, int[] layers, int[] rectangleStarts, int[] rectangles,
/// int[] objectStarts, int[] objects]`.
///
/// Each rectangle is four ints, `x, y, width, height`, and each object is three, `type, x, y`.
/// Room `i` owns the ints from `rectangleStarts[i]` to `rectangleStarts[i + 1]`, and the same for
/// objects.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeRooms<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObjectArray<'local> {
    with_handle(
        &mut env,
        handle,
        "LotHeader",
        |env, header: &AnyLotHeader| {
            let rooms = header.rooms();
            let layers: Vec<jint> = rooms.iter().map(|room| room.layer).collect();
            let (rectangles, rectangle_starts) = runs(rooms, |room, out| {
                for r in &room.rectangles {
                    out.extend_from_slice(&[r.x, r.y, r.width, r.height]);
                }
                Ok(())
            })?;
            let (objects, object_starts) = runs(rooms, |room, out| {
                for o in &room.objects {
                    out.extend_from_slice(&[o.kind, o.x, o.y]);
                }
                Ok(())
            })?;

            let names = string_array(env, rooms.iter().map(|room| room.name), rooms.len())?;
            let layers = int_array(env, &layers)?;
            let rectangle_starts = int_array(env, &rectangle_starts)?;
            let rectangles = int_array(env, &rectangles)?;
            let object_starts = int_array(env, &object_starts)?;
            let objects = int_array(env, &objects)?;
            object_array(
                env,
                &[
                    &names,
                    &layers,
                    &rectangle_starts,
                    &rectangles,
                    &object_starts,
                    &objects,
                ],
            )
        },
    )
}

/// Returns every building's room indices as `[int[] starts, int[] rooms]`.
///
/// Building `i` owns `rooms[starts[i]..starts[i + 1]]`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_LotHeader_nativeBuildings<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObjectArray<'local> {
    with_handle(
        &mut env,
        handle,
        "LotHeader",
        |env, header: &AnyLotHeader| {
            let (rooms, starts) =
                runs(header.buildings(), |building, out| {
                    for index in &building.room_indices {
                        out.push(jint::try_from(*index).map_err(|_| {
                            Failure::Argument(format!("room index {index} too large"))
                        })?);
                    }
                    Ok(())
                })?;
            let starts = int_array(env, &starts)?;
            let rooms = int_array(env, &rooms)?;
            object_array(env, &[&starts, &rooms])
        },
    )
}

// ---------------------------------------------------------------------------------------------
// Cell
// ---------------------------------------------------------------------------------------------

/// Parses a `world_*.lotpack` file and returns a handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Cell_nativeParsePath<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
    x: jint,
    y: jint,
    build: jint,
    min_layer: jint,
    max_layer: jint,
) -> jlong {
    run(&mut env, |env| {
        let path = path.try_to_string(env)?;
        let build = build_from_ordinal(build)?;
        let bytes = read_file(&path)?;
        Ok(into_handle(AnyCell::parse(
            build, x, y, &bytes, min_layer, max_layer,
        )?))
    })
}

/// Parses a `world_*.lotpack` from a byte array and returns a handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Cell_nativeParseBytes<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    data: JByteArray<'local>,
    x: jint,
    y: jint,
    build: jint,
    min_layer: jint,
    max_layer: jint,
) -> jlong {
    run(&mut env, |env| {
        let bytes = env.convert_byte_array(&data)?;
        let build = build_from_ordinal(build)?;
        Ok(into_handle(AnyCell::parse(
            build, x, y, &bytes, min_layer, max_layer,
        )?))
    })
}

/// Frees a cell handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Cell_nativeFree<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    // SAFETY: Java only ever passes a handle from `nativeParse*`, exactly once.
    unsafe { free::<AnyCell>(handle) };
}

/// Returns the cell's coordinates and sizes as `[x, y, squareCount, tileCount]`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Cell_nativeSummary<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JIntArray<'local> {
    with_handle(&mut env, handle, "Cell", |env, cell: &AnyCell| {
        let values = [
            cell.x(),
            cell.y(),
            count(cell.len(), "square count")?,
            count(cell.tiles().len(), "tile count")?,
        ];
        int_array(env, &values)
    })
}

/// Returns the tile indices of every square in the cell, concatenated into one array.
///
/// `nativeOffsets` says where each square's tiles start.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Cell_nativeTiles<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JIntArray<'local> {
    with_handle(&mut env, handle, "Cell", |env, cell: &AnyCell| {
        int_array(env, cell.tiles())
    })
}

/// Returns where each square's tiles start in `nativeTiles`, plus the total at the end.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Cell_nativeOffsets<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    with_handle(&mut env, handle, "Cell", |env, cell: &AnyCell| {
        let offsets: Vec<jlong> = cell
            .offsets()
            .iter()
            .map(|offset| jlong::try_from(*offset).unwrap_or(jlong::MAX))
            .collect();
        long_array(env, &offsets)
    })
}

/// Returns the square positions, five ints per square: `block, layer, x, y, room`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Cell_nativeSquares<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JIntArray<'local> {
    with_handle(&mut env, handle, "Cell", |env, cell: &AnyCell| {
        let mut flat = Vec::with_capacity(cell.squares().len() * 5);
        for at in cell.squares() {
            flat.extend_from_slice(&[
                count(at.block, "block index")?,
                at.layer,
                count(at.x, "square x")?,
                count(at.y, "square y")?,
                at.room,
            ]);
        }
        int_array(env, &flat)
    })
}

/// Returns the tile indices of one square.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Cell_nativeTilesOf<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    square: jint,
) -> JIntArray<'local> {
    with_handle(&mut env, handle, "Cell", |env, cell: &AnyCell| {
        let tiles = usize::try_from(square)
            .ok()
            .and_then(|index| cell.tiles_of(index))
            .ok_or_else(|| Failure::Index(format!("square index {square} out of range")))?;
        int_array(env, tiles)
    })
}

// ---------------------------------------------------------------------------------------------
// TexturePack
// ---------------------------------------------------------------------------------------------

/// Parses a `.pack` file and returns a handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_TexturePack_nativeParsePath<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
) -> jlong {
    run(&mut env, |env| {
        let path = path.try_to_string(env)?;
        let bytes = read_file(&path)?;
        Ok(into_handle(OwnedTexturePack::parse(bytes)?))
    })
}

/// Parses a `.pack` from a byte array and returns a handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_TexturePack_nativeParseBytes<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    data: JByteArray<'local>,
) -> jlong {
    run(&mut env, |env| {
        let bytes = env.convert_byte_array(&data)?;
        Ok(into_handle(OwnedTexturePack::parse(bytes)?))
    })
}

/// Frees a texture pack handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_TexturePack_nativeFree<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    // SAFETY: Java only ever passes a handle from `nativeParse*`, exactly once.
    unsafe { free::<OwnedTexturePack>(handle) };
}

/// Returns every page except its image, as `[String[] names, int[] hasAlpha, int[] textureStarts,
/// String[] textureNames, int[] textureRects]`.
///
/// `hasAlpha` is 0 or 1 per page. Page `i` owns the textures from `textureStarts[i]` to
/// `textureStarts[i + 1]`. Each texture has eight ints in `textureRects`:
/// `x, y, w, h, ox, oy, ow, oh`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_TexturePack_nativePages<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObjectArray<'local> {
    with_handle(
        &mut env,
        handle,
        "TexturePack",
        |env, pack: &OwnedTexturePack| {
            let pages = &pack.get().pages;
            let has_alpha: Vec<jint> = pages
                .iter()
                .map(|page| jint::from(page.has_alpha))
                .collect();
            let mut texture_starts = Vec::with_capacity(pages.len() + 1);
            texture_starts.push(0);
            let mut rects = Vec::new();
            let mut texture_count = 0;
            for page in pages {
                for t in &page.textures {
                    rects.extend_from_slice(&[
                        t.x,
                        t.y,
                        t.width,
                        t.height,
                        t.offset_x,
                        t.offset_y,
                        t.original_width,
                        t.original_height,
                    ]);
                }
                texture_count += page.textures.len();
                texture_starts.push(count(texture_count, "texture count")?);
            }
            let textures = pages.iter().flat_map(|page| &page.textures);

            let names = string_array(env, pages.iter().map(|page| page.name), pages.len())?;
            let has_alpha = int_array(env, &has_alpha)?;
            let texture_names = string_array(env, textures.map(|t| t.name), texture_count)?;
            let texture_starts = int_array(env, &texture_starts)?;
            let rects = int_array(env, &rects)?;
            object_array(
                env,
                &[&names, &has_alpha, &texture_starts, &texture_names, &rects],
            )
        },
    )
}

/// Returns one page's image as PNG bytes, without decoding it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_TexturePack_nativePagePng<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    handle: jlong,
    page: jint,
) -> JByteArray<'local> {
    with_handle(
        &mut env,
        handle,
        "TexturePack",
        |env, pack: &OwnedTexturePack| {
            let page = nth(&pack.get().pages, page, "page")?;
            Ok(env.byte_array_from_slice(page.png)?)
        },
    )
}

// ---------------------------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------------------------

/// Returns a build's constants as `[cellSizeInBlocks, blockSizeInSquares, cellSizeInSquares,
/// minLayer, maxLayer, versionNumber]`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Build_nativeGeometry<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    build: jint,
) -> JIntArray<'local> {
    run(&mut env, |env| {
        let build = build_from_ordinal(build)?;
        let values = [
            count(build.cell_size_in_blocks(), "cell size in blocks")?,
            count(build.block_size_in_squares(), "block size in squares")?,
            build.cell_size_in_squares(),
            build.min_layer(),
            build.max_layer(),
            count(build.version_number(), "version number")?,
        ];
        int_array(env, &values)
    })
}

/// Returns a build's display name, such as `Build 42`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_ltdjorge_pzmap_Build_nativeName<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    build: jint,
) -> JString<'local> {
    run(&mut env, |env| {
        let build = build_from_ordinal(build)?;
        Ok(env.new_string(build.name())?)
    })
}
