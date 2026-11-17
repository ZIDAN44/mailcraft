package ai.storyteller.mailcraft;

import android.app.Activity;
import android.app.Application;
import android.app.NativeActivity;
import android.os.Bundle;

import androidx.work.BackoffPolicy;
import androidx.work.Constraints;
import androidx.work.Data;
import androidx.work.ExistingPeriodicWorkPolicy;
import androidx.work.ExistingWorkPolicy;
import androidx.work.NetworkType;
import androidx.work.OneTimeWorkRequest;
import androidx.work.PeriodicWorkRequest;
import androidx.work.WorkManager;

import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

/** Small Android lifecycle adapter; all synchronization remains in the shared Rust worker. */
public final class MailCraftApplication extends Application implements Application.ActivityLifecycleCallbacks {
    private static final AtomicBoolean NATIVE_ACTIVITY_STARTING = new AtomicBoolean();
    private static final String PERIODIC_SYNC = "mailcraft.periodic-sync";
    private static final String PERIODIC_REMINDERS = "mailcraft.periodic-reminders";
    private static final String RESUME_SYNC = "mailcraft.resume-sync";
    private static final String RESUME_REMINDERS = "mailcraft.resume-reminders";

    @Override
    public void onCreate() {
        super.onCreate();
        if (!getPackageName().equals(Application.getProcessName())) {
            return;
        }
        registerActivityLifecycleCallbacks(this);

        Constraints online = new Constraints.Builder()
                .setRequiredNetworkType(NetworkType.CONNECTED)
                .build();
        PeriodicWorkRequest periodic = new PeriodicWorkRequest.Builder(
                MailCraftSyncWorker.class, 15, TimeUnit.MINUTES)
                .setConstraints(online)
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
                .build();
        WorkManager.getInstance(this).enqueueUniquePeriodicWork(
                PERIODIC_SYNC, ExistingPeriodicWorkPolicy.KEEP, periodic);

        PeriodicWorkRequest reminders = new PeriodicWorkRequest.Builder(
                MailCraftSyncWorker.class, 15, TimeUnit.MINUTES)
                .setInputData(new Data.Builder().putBoolean("remindersOnly", true).build())
                .build();
        WorkManager.getInstance(this).enqueueUniquePeriodicWork(
                PERIODIC_REMINDERS, ExistingPeriodicWorkPolicy.KEEP, reminders);
    }

    public static void noteNativeActivityStarting() {
        NATIVE_ACTIVITY_STARTING.set(true);
    }

    public static boolean isNativeActivityStarting() {
        return NATIVE_ACTIVITY_STARTING.get();
    }

    private void queueResumeSync() {
        Constraints online = new Constraints.Builder()
                .setRequiredNetworkType(NetworkType.CONNECTED)
                .build();
        OneTimeWorkRequest request = new OneTimeWorkRequest.Builder(MailCraftSyncWorker.class)
                .setConstraints(online)
                // Give NativeActivity time to checkpoint drafts and release UI-side work first.
                .setInitialDelay(10, TimeUnit.SECONDS)
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
                .build();
        WorkManager.getInstance(this).enqueueUniqueWork(
                RESUME_SYNC, ExistingWorkPolicy.KEEP, request);

        OneTimeWorkRequest reminders = new OneTimeWorkRequest.Builder(MailCraftSyncWorker.class)
                .setInputData(new Data.Builder().putBoolean("remindersOnly", true).build())
                .setInitialDelay(10, TimeUnit.SECONDS)
                .build();
        WorkManager.getInstance(this).enqueueUniqueWork(
                RESUME_REMINDERS, ExistingWorkPolicy.KEEP, reminders);
    }

    @Override
    public void onActivityPaused(Activity activity) {
        if (activity instanceof NativeActivity) {
            queueResumeSync();
        }
    }

    @Override public void onActivityCreated(Activity activity, Bundle state) {}
    @Override public void onActivityStarted(Activity activity) {}
    @Override public void onActivityResumed(Activity activity) {}
    @Override public void onActivitySaveInstanceState(Activity activity, Bundle state) {}
    @Override public void onActivityStopped(Activity activity) {}
    @Override
    public void onActivityDestroyed(Activity activity) {
        if (activity instanceof NativeActivity) {
            NATIVE_ACTIVITY_STARTING.set(false);
        }
    }
}
