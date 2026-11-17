package io.crates.keyring;

import android.content.Context;

/** JNI entry point expected by android-native-keyring-store for headless Android workers. */
public final class Keyring {
    private Keyring() {}

    public static final class Companion {
        private Companion() {}

        public static native void initializeNdkContext(Context context);
    }
}
