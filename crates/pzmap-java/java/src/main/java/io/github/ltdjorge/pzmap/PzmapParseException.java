package io.github.ltdjorge.pzmap;

import java.io.IOException;

/**
 * Thrown when map data was read but is not valid for the format and build.
 *
 * <p>It extends {@link IOException}, so code that already handles unreadable files also handles
 * malformed ones. Catch this class first if you need to tell the two apart.
 *
 * <p>{@link #getOffset()} and {@link #getFormat()} give where parsing failed and in which format,
 * as separate values from the message.
 */
public final class PzmapParseException extends IOException {

    private static final long serialVersionUID = 1L;

    /** The byte offset where parsing failed, or -1 if unknown. */
    private final long offset;

    /** The format that was being read. */
    private final String format;

    /**
     * Creates the exception. The native library calls this constructor.
     *
     * <p>The native code looks this constructor up by its parameter types. If you change them, the
     * code still compiles, but parse errors are then reported as a plain {@link IOException}.
     *
     * @param message a description of the failure
     * @param offset the byte offset in the input where the failure was found, or -1 if unknown
     * @param format the format being read: {@code "lotheader"}, {@code "lotpack"} or
     *     {@code "texture pack"}
     */
    public PzmapParseException(String message, long offset, String format) {
        super(message);
        this.offset = offset;
        this.format = format;
    }

    /**
     * Returns the byte offset in the input where the failure was found.
     *
     * @return the offset from the start of the file or byte array, or -1 if unknown
     */
    public long getOffset() {
        return offset;
    }

    /**
     * Returns the format that was being read.
     *
     * @return {@code "lotheader"}, {@code "lotpack"} or {@code "texture pack"}
     */
    public String getFormat() {
        return format;
    }
}
