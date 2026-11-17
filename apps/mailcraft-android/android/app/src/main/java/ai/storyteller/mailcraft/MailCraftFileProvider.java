package ai.storyteller.mailcraft;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.util.Base64;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.io.FileNotFoundException;
import java.nio.charset.StandardCharsets;

/** Grants another app temporary, read-only access to a MailCraft attachment being opened. */
public final class MailCraftFileProvider extends ContentProvider {
    private static final int MAX_ENCODED_PATH_LENGTH = 8192;
    private static final int MAX_QUERY_COLUMNS = 32;

    public static Uri uriFor(Context context, String path) throws FileNotFoundException {
        File file = checkedFile(context, path);
        String encodedPath = Base64.encodeToString(file.getPath().getBytes(StandardCharsets.UTF_8), Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
        return new Uri.Builder()
                .scheme("content")
                .authority(context.getPackageName() + ".files")
                .appendPath("open")
                .appendPath(encodedPath)
                .build();
    }

    private static File checkedFile(Context context, String path) throws FileNotFoundException {
        try {
            File file = new File(path).getCanonicalFile();
            File directory = file.getParentFile();
            File ownerDirectory = directory == null ? null : directory.getParentFile();
            if (directory == null || ownerDirectory == null || !"open".equals(directory.getName()) || !"mailcraft".equals(ownerDirectory.getName()) || !file.isFile()
                    || !(isInside(context.getDataDir(), file) || isInside(context.getCacheDir(), file) || isInside(context.getFilesDir(), file)
                    || isInside(context.getNoBackupFilesDir(), file))) {
                throw new FileNotFoundException("The requested file is outside MailCraft's attachment cache");
            }
            return file;
        } catch (java.io.IOException error) {
            FileNotFoundException failure = new FileNotFoundException("MailCraft could not locate the attachment");
            failure.initCause(error);
            throw failure;
        }
    }

    private static boolean isInside(File root, File file) throws java.io.IOException {
        String rootPath = root.getCanonicalPath() + File.separator;
        return file.getCanonicalPath().startsWith(rootPath);
    }

    private File fileFor(Uri uri) throws FileNotFoundException {
        if (uri == null || uri.getPathSegments().size() != 2 || !"open".equals(uri.getPathSegments().get(0))) {
            throw new FileNotFoundException("Invalid MailCraft attachment URI");
        }
        try {
            String encodedPath = uri.getPathSegments().get(1);
            if (encodedPath.length() > MAX_ENCODED_PATH_LENGTH) {
                throw new FileNotFoundException("Invalid MailCraft attachment URI");
            }
            byte[] path = Base64.decode(encodedPath, Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
            return checkedFile(getContext(), new String(path, StandardCharsets.UTF_8));
        } catch (IllegalArgumentException error) {
            FileNotFoundException failure = new FileNotFoundException("Invalid MailCraft attachment URI");
            failure.initCause(error);
            throw failure;
        }
    }

    @Override
    public boolean onCreate() {
        return getContext() != null;
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) {
            throw new FileNotFoundException("MailCraft attachments are read-only");
        }
        return ParcelFileDescriptor.open(fileFor(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override
    public String getType(Uri uri) {
        try {
            String extension = MimeTypeMap.getFileExtensionFromUrl(fileFor(uri).getName());
            String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension.toLowerCase(java.util.Locale.ROOT));
            return mime == null ? "application/octet-stream" : mime;
        } catch (FileNotFoundException error) {
            return null;
        }
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        final File file;
        try {
            file = fileFor(uri);
        } catch (FileNotFoundException error) {
            return null;
        }
        String[] columns;
        if (projection == null) {
            columns = new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        } else {
            int length = Math.min(projection.length, MAX_QUERY_COLUMNS);
            columns = new String[length];
            for (int index = 0; index < length; index++) {
                columns[index] = projection[index] == null ? "" : projection[index];
            }
        }
        MatrixCursor cursor = new MatrixCursor(columns, 1);
        Object[] row = new Object[columns.length];
        for (int index = 0; index < columns.length; index++) {
            if (OpenableColumns.DISPLAY_NAME.equals(columns[index])) {
                row[index] = file.getName();
            } else if (OpenableColumns.SIZE.equals(columns[index])) {
                row[index] = file.length();
            }
        }
        cursor.addRow(row);
        return cursor;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        return null;
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        return 0;
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        return 0;
    }
}
