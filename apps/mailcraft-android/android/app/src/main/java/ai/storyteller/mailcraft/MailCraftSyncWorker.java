package ai.storyteller.mailcraft;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.ComponentName;
import android.content.pm.PackageManager;
import android.content.SharedPreferences;
import android.os.Build;
import android.util.Log;

import androidx.work.Worker;
import androidx.work.WorkerParameters;
import androidx.work.Data;
import androidx.work.ExistingWorkPolicy;
import androidx.work.OneTimeWorkRequest;
import androidx.work.Constraints;
import androidx.work.NetworkType;
import androidx.work.WorkManager;
import androidx.work.multiprocess.RemoteListenableDelegatingWorker;
import androidx.work.multiprocess.RemoteListenableWorker;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Map;

/** Runs one bounded Rust sync pass when Android grants background execution time. */
public final class MailCraftSyncWorker extends Worker {
    private static final String TAG = "MailCraftSync";
    private static final String MAIL_CHANNEL = "ai.storyteller.mailcraft.mail";
    private static final String REMINDER_PREFS = "mailcraft.reminder-notifications";
    private static final String REMINDER_PREFIX = "sent.";
    private static final Object REMINDER_LOCK = new Object();
    static final String REMOTE_SYNC = "mailcraft.remote-sync";
    static final String REMOTE_REMINDERS = "mailcraft.remote-reminders";

    public MailCraftSyncWorker(Context context, WorkerParameters parameters) {
        super(context, parameters);
    }

    private native int runNativeSync();
    private native boolean hasActivePool();

    /** Show a privacy-safe summary from a completed headless pass, if Android permits it. */
    public void postNewMail(String title, String body, String messageKey) {
        postNewMail(getApplicationContext(), title, body, messageKey);
    }

    public static void postNewMail(Context context, String title, String body, String messageKey) {
        try {
            if (Build.VERSION.SDK_INT >= 33
                    && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                return;
            }
            NotificationManager manager = context.getSystemService(NotificationManager.class);
            if (manager == null || !manager.areNotificationsEnabled()) {
                return;
            }
            NotificationChannel channel = manager.getNotificationChannel(MAIL_CHANNEL);
            if (channel == null) {
                channel = new NotificationChannel(MAIL_CHANNEL, title, NotificationManager.IMPORTANCE_DEFAULT);
                manager.createNotificationChannel(channel);
            }
            if (channel.getImportance() == NotificationManager.IMPORTANCE_NONE) {
                return;
            }

            boolean hasMessage = messageKey != null && !messageKey.isEmpty() && messageKey.length() <= 1024
                    && messageKey.chars().noneMatch(Character::isISOControl);
            Intent openInbox = new Intent(context, MailCraftActivity.class)
                    .putExtra("mailcraft.notification", hasMessage ? "message" : "inbox")
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP);
            if (hasMessage) {
                openInbox.putExtra("mailcraft.key", messageKey);
            }
            PendingIntent pending = PendingIntent.getActivity(
                    context, 7101, openInbox, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            Notification notification = new Notification.Builder(context, MAIL_CHANNEL)
                    .setSmallIcon(context.getApplicationInfo().icon)
                    .setContentTitle(title)
                    .setContentText(body)
                    .setVisibility(Notification.VISIBILITY_PRIVATE)
                    .setAutoCancel(true)
                    .setContentIntent(pending)
                    .build();
            manager.notify(7101, notification);
        } catch (RuntimeException error) {
            // A notification failure must not repeat a completed server sync.
            Log.i(TAG, "Could not post the background mail notification", error);
        }
    }

    /** Post a due event/task reminder once for this occurrence and snooze time. */
    public boolean postReminder(String title, String body, String account, String uid, long occurrence, long due, boolean task, String identity) {
        return postReminder(getApplicationContext(), title, body, account, uid, occurrence, due, task, identity);
    }

    public static boolean postReminder(Context context, String title, String body, String account, String uid, long occurrence, long due, boolean task, String identity) {
        synchronized (REMINDER_LOCK) {
            try {
                if (identity == null || identity.length() > 4096 || account == null || account.length() > 256
                        || uid == null || uid.length() > 1024 || identity.isEmpty() || account.isEmpty() || uid.isEmpty()
                        || account.chars().anyMatch(Character::isISOControl) || uid.chars().anyMatch(Character::isISOControl)) {
                    return false;
                }
                if (Build.VERSION.SDK_INT >= 33
                        && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                    return false;
                }
                NotificationManager manager = context.getSystemService(NotificationManager.class);
                if (manager == null || !manager.areNotificationsEnabled()) {
                    return false;
                }
                String channelId = task ? "ai.storyteller.mailcraft.tasks" : "ai.storyteller.mailcraft.calendar";
                NotificationChannel channel = manager.getNotificationChannel(channelId);
                if (channel == null) {
                    channel = new NotificationChannel(channelId, title, NotificationManager.IMPORTANCE_DEFAULT);
                    manager.createNotificationChannel(channel);
                }
                if (channel.getImportance() == NotificationManager.IMPORTANCE_NONE) {
                    return false;
                }

                SharedPreferences preferences = context.getSharedPreferences(REMINDER_PREFS, Context.MODE_PRIVATE);
                String marker = reminderMarker(identity);
                if (preferences.contains(marker)) {
                    return true;
                }
                int notificationId = 1_500_000_000 + (identity.hashCode() & 0x0fffffff);
                Intent openReminder = new Intent(context, MailCraftActivity.class)
                        .putExtra("mailcraft.notification", task ? "task" : "event")
                        .putExtra("mailcraft.account", account)
                        .putExtra("mailcraft.uid", uid)
                        .putExtra("mailcraft.occurrence", occurrence)
                        .putExtra("mailcraft.due", due)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP);
                PendingIntent pending = PendingIntent.getActivity(
                        context, notificationId, openReminder, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
                Notification notification = new Notification.Builder(context, channelId)
                        .setSmallIcon(context.getApplicationInfo().icon)
                        .setContentTitle(title)
                        .setContentText(body)
                        .setVisibility(Notification.VISIBILITY_PRIVATE)
                        .setOnlyAlertOnce(true)
                        .setAutoCancel(true)
                        .setContentIntent(pending)
                        .build();
                manager.notify(notificationId, notification);
                rememberReminder(context, marker);
                return true;
            } catch (RuntimeException error) {
                Log.i(TAG, "Could not post the background reminder", error);
                return false;
            }
        }
    }

    /** Used by foreground Rust notifications to avoid repeating a background reminder. */
    public static boolean wasReminderPosted(Context context, String identity) {
        if (identity == null || identity.length() > 4096) {
            return false;
        }
        synchronized (REMINDER_LOCK) {
            try {
                return context.getSharedPreferences(REMINDER_PREFS, Context.MODE_PRIVATE).contains(reminderMarker(identity));
            } catch (RuntimeException error) {
                Log.i(TAG, "Could not check a previously posted reminder", error);
                return false;
            }
        }
    }

    /** Remember a foreground reminder after Android accepted it. */
    public static void markReminderPosted(Context context, String identity) {
        if (identity == null || identity.length() > 4096) {
            return;
        }
        synchronized (REMINDER_LOCK) {
            try {
                rememberReminder(context, reminderMarker(identity));
            } catch (RuntimeException error) {
                Log.i(TAG, "Could not save reminder notification state", error);
            }
        }
    }

    private static String reminderMarker(String identity) {
        try {
            byte[] digest = MessageDigest.getInstance("SHA-256").digest(identity.getBytes(StandardCharsets.UTF_8));
            StringBuilder marker = new StringBuilder(REMINDER_PREFIX.length() + digest.length * 2).append(REMINDER_PREFIX);
            for (byte value : digest) {
                marker.append(String.format(java.util.Locale.ROOT, "%02x", value & 0xff));
            }
            return marker.toString();
        } catch (java.security.NoSuchAlgorithmException error) {
            throw new IllegalStateException("Android is missing SHA-256", error);
        }
    }

    private static void rememberReminder(Context context, String marker) {
        SharedPreferences preferences = context.getSharedPreferences(REMINDER_PREFS, Context.MODE_PRIVATE);
        preferences.edit().putLong(marker, System.currentTimeMillis()).apply();
        Map<String, ?> saved = preferences.getAll();
        if (saved.size() <= 512) {
            return;
        }
        ArrayList<String> keys = new ArrayList<>();
        for (Map.Entry<String, ?> entry : saved.entrySet()) {
            if (entry.getKey().startsWith(REMINDER_PREFIX) && entry.getValue() instanceof Long) {
                keys.add(entry.getKey());
            }
        }
        keys.sort((left, right) -> Long.compare(preferences.getLong(left, 0), preferences.getLong(right, 0)));
        SharedPreferences.Editor editor = preferences.edit();
        int remove = Math.max(0, keys.size() - 384);
        for (int index = 0; index < remove; index++) {
            editor.remove(keys.get(index));
        }
        editor.apply();
    }


    @Override
    public Result doWork() {
        if (isStopped()) {
            return Result.retry();
        }
        try {
            System.loadLibrary("mailcraft_android");
            if (!hasActivePool()) {
                if (MailCraftApplication.isNativeActivityStarting()) {
                    return Result.retry();
                }
                enqueueRemoteWork();
                return Result.success();
            }
            int result = runNativeSync();
            if (isStopped()) {
                return Result.retry();
            }
            return result == 0 ? Result.success() : Result.retry();
        } catch (LinkageError | RuntimeException error) {
            Log.w(TAG, "Background sync could not start; Android will retry", error);
            return Result.retry();
        }
    }

    private void enqueueRemoteWork() {
        boolean remindersOnly = getInputData().getBoolean("remindersOnly", false);
        String workName = remindersOnly ? REMOTE_REMINDERS : REMOTE_SYNC;
        ComponentName remoteService = new ComponentName(getApplicationContext(), androidx.work.multiprocess.RemoteWorkerService.class);
        Data input = new Data.Builder()
                .putBoolean("remindersOnly", remindersOnly)
                .putString(RemoteListenableWorker.ARGUMENT_PACKAGE_NAME, remoteService.getPackageName())
                .putString(RemoteListenableWorker.ARGUMENT_CLASS_NAME, remoteService.getClassName())
                .putString(
                        "androidx.work.multiprocess.RemoteListenableDelegatingWorker.ARGUMENT_REMOTE_LISTENABLE_WORKER_NAME",
                        MailCraftRemoteSyncWorker.class.getName())
                .build();
        OneTimeWorkRequest.Builder requestBuilder = new OneTimeWorkRequest.Builder(RemoteListenableDelegatingWorker.class)
                .setInputData(input);
        if (!remindersOnly) {
            requestBuilder.setConstraints(new Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build());
        }
        OneTimeWorkRequest request = requestBuilder.build();
        WorkManager.getInstance(getApplicationContext()).enqueueUniqueWork(workName, ExistingWorkPolicy.KEEP, request);
    }

    public static void cancelRemoteWork(Context context) {
        try {
            WorkManager manager = WorkManager.getInstance(context);
            manager.cancelUniqueWork(REMOTE_SYNC);
            manager.cancelUniqueWork(REMOTE_REMINDERS);
        } catch (RuntimeException error) {
            Log.w(TAG, "Could not stop background work before opening the cache", error);
        }
    }
}
