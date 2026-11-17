package ai.storyteller.mailcraft;

import android.Manifest;
import android.app.NativeActivity;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.view.Window;
import android.view.WindowInsetsController;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/** Keeps the running NativeActivity alive while a second instance stages an incoming Android intent. */
public final class MailCraftActivity extends NativeActivity {
    private static final String RELAY_MARKER = "mailcraft.handoff-relay";
    private static final int NOTIFICATION_PERMISSION_REQUEST = 4107;
    private static final int DOCUMENT_REQUEST = 4108;
    private volatile boolean notificationPermissionPending;
    private int activeDocumentRequest;
    private boolean activeDocumentAllowsMultiple;
    private Object systemBackCallback;

    private static native void nativeSystemBack();
    private static native void nativeDocumentResult(int requestId, int resultCode, String uri, String name,
            String mime, long size, String error);
    private static native void nativeDocumentBatchResult(int requestId, int resultCode, String documents, String error);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        MailCraftApplication.noteNativeActivityStarting();
        MailCraftSyncWorker.cancelRemoteWork(getApplicationContext());
        super.onCreate(savedInstanceState);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            systemBackCallback = Api33Back.register(this);
        }
    }

    @Override
    protected void onDestroy() {
        if (systemBackCallback != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            Api33Back.unregister(this, systemBackCallback);
            systemBackCallback = null;
        }
        super.onDestroy();
    }

    /** Android requires permission prompts to be requested on the Activity's UI thread. */
    public void requestMailCraftNotificationPermission() {
        notificationPermissionPending = true;
        runOnUiThread(() -> {
            try {
                requestPermissions(new String[] { Manifest.permission.POST_NOTIFICATIONS }, NOTIFICATION_PERMISSION_REQUEST);
            } catch (RuntimeException error) {
                notificationPermissionPending = false;
            }
        });
    }

    public boolean isMailCraftNotificationPermissionPending() {
        return notificationPermissionPending;
    }

    /** Open Android's document provider and return the chosen URI to Rust. */
    public void startMailCraftOpenDocument(int requestId, String mime, boolean allowMultiple) {
        runOnUiThread(() -> {
            if (activeDocumentRequest != 0) {
                nativeDocumentResult(requestId, RESULT_CANCELED, null, null, null, -1,
                        "Another Android document window is already open");
                return;
            }
            activeDocumentRequest = requestId;
            activeDocumentAllowsMultiple = allowMultiple;
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType(mime == null || mime.isEmpty() ? "*/*" : mime);
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, allowMultiple);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
            try {
                startActivityForResult(intent, DOCUMENT_REQUEST);
            } catch (RuntimeException error) {
                activeDocumentRequest = 0;
                activeDocumentAllowsMultiple = false;
                nativeDocumentResult(requestId, RESULT_CANCELED, null, null, null, -1,
                        "Android could not open the document picker");
            }
        });
    }

    /** Ask Android where to save a document, then stream its bytes to the returned URI in Rust. */
    public void startMailCraftCreateDocument(int requestId, String name, String mime) {
        runOnUiThread(() -> {
            if (activeDocumentRequest != 0) {
                nativeDocumentResult(requestId, RESULT_CANCELED, null, null, null, -1,
                        "Another Android document window is already open");
                return;
            }
            activeDocumentRequest = requestId;
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType(mime == null || mime.isEmpty() ? "application/octet-stream" : mime);
            intent.putExtra(Intent.EXTRA_TITLE, name == null || name.isEmpty() ? "MailCraft file" : name);
            intent.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_READ_URI_PERMISSION);
            try {
                startActivityForResult(intent, DOCUMENT_REQUEST);
            } catch (RuntimeException error) {
                activeDocumentRequest = 0;
                nativeDocumentResult(requestId, RESULT_CANCELED, null, null, null, -1,
                        "Android could not open the save dialog");
            }
        });
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != DOCUMENT_REQUEST) {
            return;
        }
        int requestId = activeDocumentRequest;
        boolean allowMultiple = activeDocumentAllowsMultiple;
        activeDocumentRequest = 0;
        activeDocumentAllowsMultiple = false;
        if (requestId == 0 || resultCode != RESULT_OK || data == null) {
            if (requestId != 0) {
                nativeDocumentResult(requestId, resultCode, null, null, null, -1, null);
            }
            return;
        }
        if (allowMultiple) {
            JSONArray documents = new JSONArray();
            android.content.ClipData clips = data.getClipData();
            int count = clips == null ? 0 : clips.getItemCount();
            if (count == 0 && data.getData() != null) {
                count = 1;
            }
            if (count == 0 || count > 50) {
                nativeDocumentBatchResult(requestId, resultCode, null, "Choose between one and 50 files");
                return;
            }
            try {
                for (int index = 0; index < count; index++) {
                    Uri selected = clips == null ? data.getData() : clips.getItemAt(index).getUri();
                    if (selected == null) {
                        throw new JSONException("The selection contains an empty document");
                    }
                    documents.put(documentDetails(selected));
                }
                nativeDocumentBatchResult(requestId, resultCode, documents.toString(), null);
            } catch (JSONException error) {
                nativeDocumentBatchResult(requestId, resultCode, null, "Android could not read the document selection");
            }
            return;
        }
        Uri uri = data.getData();
        if (uri == null) {
            nativeDocumentResult(requestId, resultCode, null, null, null, -1,
                    "Android did not return the selected document");
            return;
        }
        String name = null;
        String mime = null;
        long size = -1;
        try {
            mime = getContentResolver().getType(uri);
            try (Cursor cursor = getContentResolver().query(uri,
                    new String[] { OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE }, null, null, null)) {
                if (cursor != null && cursor.moveToFirst()) {
                    int nameColumn = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                    int sizeColumn = cursor.getColumnIndex(OpenableColumns.SIZE);
                    if (nameColumn >= 0 && !cursor.isNull(nameColumn)) {
                        name = cursor.getString(nameColumn);
                    }
                    if (sizeColumn >= 0 && !cursor.isNull(sizeColumn)) {
                        size = cursor.getLong(sizeColumn);
                    }
                }
            }
        } catch (RuntimeException ignored) {
            // Some document providers omit metadata; Rust can still read the URI with size caps.
        }
        String uriText = uri.toString();
        if (uriText.length() > 8192) {
            nativeDocumentResult(requestId, resultCode, null, null, null, -1,
                    "The selected document address is too long");
            return;
        }
        if (name != null && name.length() > 1024) {
            name = name.substring(0, 1024);
        }
        if (mime != null && mime.length() > 256) {
            mime = mime.substring(0, 256);
        }
        nativeDocumentResult(requestId, resultCode, uriText, name, mime, size, null);
    }

    private JSONObject documentDetails(Uri uri) throws JSONException {
        String name = null;
        String mime = null;
        long size = -1;
        try {
            mime = getContentResolver().getType(uri);
            try (Cursor cursor = getContentResolver().query(uri,
                    new String[] { OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE }, null, null, null)) {
                if (cursor != null && cursor.moveToFirst()) {
                    int nameColumn = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                    int sizeColumn = cursor.getColumnIndex(OpenableColumns.SIZE);
                    if (nameColumn >= 0 && !cursor.isNull(nameColumn)) {
                        name = cursor.getString(nameColumn);
                    }
                    if (sizeColumn >= 0 && !cursor.isNull(sizeColumn)) {
                        size = cursor.getLong(sizeColumn);
                    }
                }
            }
        } catch (RuntimeException ignored) {
            // Some document providers omit metadata; Rust can still read the URI with size caps.
        }
        String uriText = uri.toString();
        if (uriText.length() > 8192) {
            throw new JSONException("The selected document address is too long");
        }
        if (name != null && name.length() > 1024) {
            name = name.substring(0, 1024);
        }
        if (mime != null && mime.length() > 256) {
            mime = mime.substring(0, 256);
        }
        JSONObject document = new JSONObject();
        document.put("uri", uriText);
        document.put("name", name);
        document.put("mime", mime);
        document.put("size", size);
        return document;
    }

    /** Apply system bar colors on Android's UI thread, including on the render-thread NativeActivity. */
    public void setMailCraftSystemBars(int statusColor, int navigationColor, boolean statusDarkIcons, boolean navigationDarkIcons) {
        runOnUiThread(() -> {
            try {
                Window window = getWindow();
                window.setStatusBarColor(statusColor);
                window.setNavigationBarColor(navigationColor);
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    WindowInsetsController controller = window.getDecorView().getWindowInsetsController();
                    if (controller != null) {
                        int status = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS;
                        int navigation = WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                        int appearance = (statusDarkIcons ? status : 0) | (navigationDarkIcons ? navigation : 0);
                        controller.setSystemBarsAppearance(appearance, status | navigation);
                    }
                }
            } catch (RuntimeException error) {
                // A window can be detaching while Android finishes a theme update.
            }
        });
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == NOTIFICATION_PERMISSION_REQUEST) {
            notificationPermissionPending = false;
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        if (!isHandoff(intent) || intent.getBooleanExtra(RELAY_MARKER, false)) {
            return;
        }

        Intent relay = new Intent(intent);
        relay.setClass(this, MailCraftActivity.class);
        relay.setSelector(null);
        relay.putExtra(RELAY_MARKER, true);
        int uriGrants = intent.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION
                | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION
                | Intent.FLAG_GRANT_PREFIX_URI_PERMISSION);
        relay.setFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_MULTIPLE_TASK | uriGrants);
        startActivity(relay);
    }

    private static boolean isHandoff(Intent intent) {
        if (intent == null) {
            return false;
        }
        if (intent.hasExtra("mailcraft.notification")) {
            return true;
        }
        String action = intent.getAction();
        return Intent.ACTION_VIEW.equals(action)
                || Intent.ACTION_SEND.equals(action)
                || Intent.ACTION_SEND_MULTIPLE.equals(action)
                || Intent.ACTION_SENDTO.equals(action);
    }

    /** Keep API 33 classes out of NativeActivity's load path on supported Android 12 devices. */
    private static final class Api33Back {
        private static Object register(MailCraftActivity activity) {
            android.window.OnBackInvokedCallback callback = MailCraftActivity::nativeSystemBack;
            activity.getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback);
            return callback;
        }

        private static void unregister(MailCraftActivity activity, Object callback) {
            if (callback instanceof android.window.OnBackInvokedCallback backCallback) {
                activity.getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(backCallback);
            }
        }
    }
}
