// Synthetic adb/app_process harness. No Activity/UI, no user data or credentials.
public final class TaypeerNative {
    private static native int run(Object context, String[] args);
    public static void main(String[] args) throws Exception {
        Class.forName("android.os.Looper").getMethod("prepareMainLooper").invoke(null);
        Class<?> thread = Class.forName("android.app.ActivityThread");
        Object instance = thread.getMethod("systemMain").invoke(null);
        Object context = thread.getMethod("getSystemContext").invoke(instance);
        System.load("/data/local/tmp/taypeer-spike-suite/libtaypeer_encrypted_sync_spike.so");
        System.exit(run(context, args));
    }
}
