package app.craftlauncher.android;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;

import java.io.File;
import java.io.FileNotFoundException;

/**
 * Serves APK files the launcher downloaded into the app cache dir to the
 * system package installer. The Rust side fires an ACTION_VIEW intent for
 * {@code content://app.craftlauncher.android.apkprovider/<name.apk>}; this
 * provider opens that file read-only. Names are strictly validated so only
 * .apk files from our own cache dir are ever served.
 */
public class ApkProvider extends ContentProvider {
    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        String name = uri.getLastPathSegment();
        if (name == null || !name.endsWith(".apk")
                || name.contains("/") || name.contains("\\") || name.contains("..")) {
            throw new FileNotFoundException("refused apk uri");
        }
        File file = new File(getContext().getCacheDir(), name);
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override
    public String getType(Uri uri) {
        return "application/vnd.android.package-archive";
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection,
                        String[] selectionArgs, String sortOrder) {
        return null;
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
    public int update(Uri uri, ContentValues values, String selection,
                       String[] selectionArgs) {
        return 0;
    }
}
