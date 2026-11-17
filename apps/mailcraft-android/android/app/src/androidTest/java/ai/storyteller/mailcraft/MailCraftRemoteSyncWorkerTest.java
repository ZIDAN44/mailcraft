package ai.storyteller.mailcraft;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;

import android.content.ComponentName;
import android.content.Context;
import android.os.SystemClock;

import androidx.test.core.app.ApplicationProvider;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.work.Constraints;
import androidx.work.Data;
import androidx.work.NetworkType;
import androidx.work.OneTimeWorkRequest;
import androidx.work.WorkInfo;
import androidx.work.WorkManager;
import androidx.work.multiprocess.RemoteListenableDelegatingWorker;
import androidx.work.multiprocess.RemoteListenableWorker;
import androidx.work.multiprocess.RemoteWorkerService;

import org.junit.Test;
import org.junit.runner.RunWith;

import java.util.concurrent.TimeUnit;

@RunWith(AndroidJUnit4.class)
public final class MailCraftRemoteSyncWorkerTest {
    @Test
    public void coldStartSyncInitializesKeystoreInTheRemoteProcess() throws Exception {
        Context context = ApplicationProvider.getApplicationContext();
        ComponentName remoteService = new ComponentName(context, RemoteWorkerService.class);
        Data input = new Data.Builder()
                .putBoolean("remindersOnly", false)
                .putString(RemoteListenableWorker.ARGUMENT_PACKAGE_NAME, remoteService.getPackageName())
                .putString(RemoteListenableWorker.ARGUMENT_CLASS_NAME, remoteService.getClassName())
                .putString(
                        "androidx.work.multiprocess.RemoteListenableDelegatingWorker.ARGUMENT_REMOTE_LISTENABLE_WORKER_NAME",
                        MailCraftRemoteSyncWorker.class.getName())
                .build();
        OneTimeWorkRequest request = new OneTimeWorkRequest.Builder(RemoteListenableDelegatingWorker.class)
                .setInputData(input)
                .setConstraints(new Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build();
        WorkManager manager = WorkManager.getInstance(context);
        manager.enqueue(request).getResult().get(10, TimeUnit.SECONDS);

        long deadline = SystemClock.uptimeMillis() + TimeUnit.SECONDS.toMillis(60);
        WorkInfo info = null;
        while (SystemClock.uptimeMillis() < deadline) {
            info = manager.getWorkInfoById(request.getId()).get(5, TimeUnit.SECONDS);
            if (info != null && info.getState().isFinished()) {
                break;
            }
            Thread.sleep(200);
        }
        assertNotNull("Remote sync work should finish", info);
        assertEquals(WorkInfo.State.SUCCEEDED, info.getState());
    }
}
