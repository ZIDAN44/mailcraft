package ai.storyteller.mailcraft;

import android.content.Context;
import android.util.Log;

import androidx.concurrent.futures.CallbackToFutureAdapter;
import androidx.work.ListenableWorker;
import androidx.work.WorkerParameters;
import androidx.work.multiprocess.RemoteListenableWorker;

import com.google.common.util.concurrent.ListenableFuture;

import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;

/** Runs cold-start Rust sync in a process isolated from NativeActivity's NDK context. */
public final class MailCraftRemoteSyncWorker extends RemoteListenableWorker {
    private static final String TAG = "MailCraftSync";
    private static final ExecutorService EXECUTOR = Executors.newCachedThreadPool(task -> {
        Thread thread = new Thread(task, "mailcraft-remote-sync");
        thread.setDaemon(true);
        return thread;
    });

    public MailCraftRemoteSyncWorker(Context context, WorkerParameters parameters) {
        super(context, parameters);
    }

    private native int runNativeSync();

    @Override
    public ListenableFuture<ListenableWorker.Result> startRemoteWork() {
        return CallbackToFutureAdapter.getFuture(completer -> {
            Future<?> task = EXECUTOR.submit(() -> {
                if (isStopped()) {
                    completer.set(ListenableWorker.Result.retry());
                    return;
                }
                try {
                    System.loadLibrary("mailcraft_android");
                    int result = runNativeSync();
                    completer.set(isStopped() || result != 0
                            ? ListenableWorker.Result.retry()
                            : ListenableWorker.Result.success());
                } catch (LinkageError | RuntimeException error) {
                    Log.w(TAG, "Cold-start background sync could not start; Android will retry", error);
                    completer.set(ListenableWorker.Result.retry());
                }
            });
            completer.addCancellationListener(() -> task.cancel(true), EXECUTOR);
            return "MailCraft isolated background sync";
        });
    }

    public void postNewMail(String title, String body, String messageKey) {
        MailCraftSyncWorker.postNewMail(getApplicationContext(), title, body, messageKey);
    }

    public boolean postReminder(
            String title,
            String body,
            String account,
            String uid,
            long occurrence,
            long due,
            boolean task,
            String identity) {
        return MailCraftSyncWorker.postReminder(
                getApplicationContext(), title, body, account, uid, occurrence, due, task, identity);
    }
}
