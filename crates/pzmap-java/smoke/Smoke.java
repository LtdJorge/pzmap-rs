import io.github.ltdjorge.pzmap.Page;
import io.github.ltdjorge.pzmap.TexturePack;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.List;

/**
 * Checks that a built jar loads its own native library on this platform and can call it.
 *
 * <p>The release workflow runs it against the finished jar on each platform, from source:
 *
 * <pre>java -cp pzmap-VERSION.jar Smoke.java</pre>
 */
public class Smoke {
    public static void main(String[] args) throws Exception {
        System.out.println(
                "Java " + System.getProperty("java.version") + " on "
                        + System.getProperty("os.name") + " " + System.getProperty("os.arch"));

        // A pack with one page named "p", no textures and a four-byte image.
        ByteBuffer buffer = ByteBuffer.allocate(33).order(ByteOrder.LITTLE_ENDIAN);
        buffer.put(new byte[] {'P', 'Z', 'P', 'K'});
        buffer.putInt(1); // envelope version
        buffer.putInt(1); // page count
        buffer.putInt(1).put((byte) 'p'); // page name
        buffer.putInt(0); // texture count
        buffer.putInt(1); // has alpha
        buffer.putInt(4).put(new byte[] {(byte) 0x89, 'P', 'N', 'G'});

        try (TexturePack pack = TexturePack.parse(buffer.array())) {
            List<Page> pages = pack.pages();
            if (pack.pageCount() != 1 || !pages.get(0).name().equals("p")) {
                throw new AssertionError("unexpected pages: " + pages);
            }
        }
        System.out.println("The native library loaded and parsed a texture pack.");
    }
}
