// Tiny guest-side recorder run via app_process (no APK needed):
//   adb shell CLASSPATH=/data/local/tmp/rec.dex app_process / Rec <out.raw> <seconds>
// Records 48 kHz mono s16le from AudioSource.MIC through the normal Android audio stack.
// Build: javac --release 8 -cp $ANDROID_HOME/platforms/android-35/android.jar -d OUT Rec.java
//        $ANDROID_HOME/build-tools/35.0.0/d8 --output OUT --lib $ANDROID_HOME/platforms/android-35/android.jar OUT/Rec.class
//        adb push OUT/classes.dex /data/local/tmp/rec.dex   (needs `adb root` on google_apis image)
import android.media.AudioFormat;
import android.media.AudioRecord;
import android.media.MediaRecorder;
import java.io.FileOutputStream;

public class Rec {
    public static void main(String[] args) throws Exception {
        String out = args.length > 0 ? args[0] : "/data/local/tmp/cap.raw";
        int secs = args.length > 1 ? Integer.parseInt(args[1]) : 3;
        int rate = 48000;
        int min = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT);
        AudioRecord r = new AudioRecord(MediaRecorder.AudioSource.MIC, rate,
                AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, Math.max(min, 9600) * 2);
        if (r.getState() != AudioRecord.STATE_INITIALIZED) { System.err.println("AudioRecord init failed"); System.exit(2); }
        byte[] buf = new byte[9600];
        long total = 0, want = (long) rate * 2 * secs;
        try (FileOutputStream fos = new FileOutputStream(out)) {
            r.startRecording();
            while (total < want) {
                int n = r.read(buf, 0, buf.length);
                if (n < 0) { System.err.println("read error " + n); break; }
                fos.write(buf, 0, n); total += n;
            }
        } finally { r.stop(); r.release(); }
        System.out.println("wrote " + total + " bytes to " + out);
    }
}
