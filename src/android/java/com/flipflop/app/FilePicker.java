package com.flipflop.app;

import android.app.Activity;
import android.app.Fragment;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Runs the system file picker (or the photo picker) for the native app.
 *
 * NativeActivity drops onActivityResult, so the picker is started from this
 * headless fragment instead, which gets the result and hands the picked
 * content URIs to native code ({@link #onPicked}, registered from Rust; an
 * empty array means cancelled). The class is compiled into a dex at build
 * time and loaded at runtime (see src/android.rs), so it is not part of the
 * APK's own code.
 */
public class FilePicker extends Fragment {
    private static final String TAG = "flipflop-file-picker";
    private static final int REQUEST = 0x7a11;
    private static final int MAX_IMAGES = 100;

    static native void onPicked(String[] uris);

    /** Any documents. Must be called on the UI thread. */
    public static void pick(Activity activity) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        start(activity, intent);
    }

    /**
     * Pictures from the gallery: the system photo picker on Android 13+,
     * otherwise whatever gallery app handles image content. Must be called
     * on the UI thread.
     */
    public static void pickImages(Activity activity) {
        Intent intent;
        if (Build.VERSION.SDK_INT >= 33) {
            intent = new Intent(MediaStore.ACTION_PICK_IMAGES);
            intent.setType("image/*");
            intent.putExtra(MediaStore.EXTRA_PICK_IMAGES_MAX, MAX_IMAGES);
        } else {
            intent = new Intent(Intent.ACTION_GET_CONTENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType("image/*");
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        }
        start(activity, intent);
    }

    /**
     * Copies content URIs into outDir and returns the staged absolute paths, one per
     * line. Throws with the provider's own reason (permission denied, no stream) so it
     * reaches the user, instead of the bare JNI error a failed read on the Rust side
     * would leave behind.
     *
     * Doing the copy here keeps every URI quirk in one language: the `file://` URIs
     * some vendor pickers still hand out, providers that ignore the display-name
     * query, and picked files whose names collide.
     */
    public static String copyUris(Activity activity, String[] uris, String outDir)
            throws IOException {
        ContentResolver resolver = activity.getContentResolver();
        File dir = new File(outDir);
        if (!dir.isDirectory() && !dir.mkdirs()) {
            throw new IOException("cannot create " + dir);
        }
        StringBuilder staged = new StringBuilder();
        for (String text : uris) {
            Uri uri = Uri.parse(text);
            File dest = unique(dir, displayName(resolver, uri));
            InputStream in = open(resolver, uri);
            try {
                OutputStream out = new FileOutputStream(dest);
                try {
                    byte[] buffer = new byte[64 * 1024];
                    int read;
                    while ((read = in.read(buffer)) > 0) {
                        out.write(buffer, 0, read);
                    }
                } finally {
                    out.close();
                }
            } finally {
                in.close();
            }
            staged.append(dest.getAbsolutePath()).append('\n');
        }
        return staged.toString();
    }

    /** `ContentResolver` cannot open the `file://` URIs some pickers return. */
    private static InputStream open(ContentResolver resolver, Uri uri) throws IOException {
        if ("file".equals(uri.getScheme())) {
            return new FileInputStream(new File(uri.getPath()));
        }
        InputStream in = resolver.openInputStream(uri);
        if (in == null) {
            throw new IOException("no stream for " + uri);
        }
        return in;
    }

    /** The name the picker advertises, or the URI's last path segment. */
    private static String displayName(ContentResolver resolver, Uri uri) {
        Cursor cursor = null;
        try {
            cursor = resolver.query(
                    uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null);
            if (cursor != null && cursor.moveToFirst()) {
                String name = cursor.getString(0);
                if (name != null && !name.trim().isEmpty()) {
                    return name;
                }
            }
        } catch (RuntimeException e) {
            // Providers that do not answer this fall back to the path segment.
        } finally {
            if (cursor != null) {
                cursor.close();
            }
        }
        String segment = uri.getLastPathSegment();
        return segment == null ? "shared-file" : segment;
    }

    /** `dir` plus a sanitised `name`, made unique so two picks cannot collide. */
    private static File unique(File dir, String name) {
        StringBuilder safe = new StringBuilder();
        for (int i = 0; i < name.length(); i++) {
            char c = name.charAt(i);
            safe.append(Character.isLetterOrDigit(c) || c == '.' || c == '-' || c == '_' ? c : '_');
        }
        if (safe.length() == 0) {
            safe.append("shared-file");
        }
        File file = new File(dir, safe.toString());
        for (int n = 1; file.exists(); n++) {
            file = new File(dir, n + "-" + safe);
        }
        return file;
    }

    private static void start(Activity activity, Intent intent) {
        FilePicker picker = new FilePicker();
        activity.getFragmentManager().beginTransaction().add(picker, TAG).commitNow();
        try {
            picker.startActivityForResult(intent, REQUEST);
        } catch (RuntimeException e) {
            // No picker for this intent on this device.
            picker.done(new String[0]);
        }
    }

    @Override
    public void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (requestCode != REQUEST) {
            return;
        }
        String[] uris = new String[0];
        if (resultCode == Activity.RESULT_OK && data != null) {
            ClipData clip = data.getClipData();
            if (clip != null) {
                uris = new String[clip.getItemCount()];
                for (int i = 0; i < uris.length; i++) {
                    uris[i] = clip.getItemAt(i).getUri().toString();
                }
            } else if (data.getData() != null) {
                uris = new String[] {data.getData().toString()};
            }
        }
        done(uris);
    }

    private void done(String[] uris) {
        if (isAdded()) {
            getFragmentManager().beginTransaction().remove(this).commitAllowingStateLoss();
        }
        onPicked(uris);
    }
}
