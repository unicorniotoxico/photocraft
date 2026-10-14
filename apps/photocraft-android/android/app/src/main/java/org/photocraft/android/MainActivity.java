package org.photocraft.android;

import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.util.Log;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.inputmethod.InputMethodManager;

import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;

/**
 * The PhotoCraft activity: Rust (libphotocraft_android.so, see src/jni_bridge.rs) draws the whole
 * app; this class only does what needs Java: the Storage Access Framework pickers, files shared
 * to the app, the soft keyboard, and opening links.
 */
public class MainActivity extends NativeActivity {
    private static final String TAG = "PhotoCraft";
    private static final int REQUEST_OPEN = 1;
    private static final int REQUEST_SAVE = 2;

    static {
        // NativeActivity loads the library too; loading it here registers the native methods
        // below with this class loader.
        System.loadLibrary("photocraft_android");
    }

    /** Picked or shared files: names and descriptors detached from their ParcelFileDescriptors. */
    private static native void nativeFilesPicked(String[] names, int[] fds);
    private static native void nativePickCancelled();
    /** {@code message}: the saved name on success, the error on failure, null when cancelled. */
    private static native void nativeSaveFinished(boolean ok, String message);

    /** The cache file waiting for the "create document" picker's answer. */
    private String pendingSave;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        hideSystemBars();
        handleIntent(getIntent());
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleIntent(intent);
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemBars();
        }
    }

    /** Back never closes the app (unsaved documents live in this process): it goes to the background. */
    @Override
    public void onBackPressed() {
        moveTaskToBack(true);
    }

    // ---- Called from Rust, on its thread --------------------------------------------------

    public void pickFiles(final boolean multiple) {
        runOnUiThread(() -> {
            Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            // Android's MIME types don't cover PSD, Krita or camera raw reliably: show every file
            // and let the importer decide.
            intent.setType("*/*");
            intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
            try {
                startActivityForResult(intent, REQUEST_OPEN);
            } catch (ActivityNotFoundException e) {
                Log.e(TAG, "no document picker", e);
                nativePickCancelled();
            }
        });
    }

    public void saveFile(final String name, final String mime, final String cachePath) {
        runOnUiThread(() -> {
            if (pendingSave != null) {
                // A second save before the first picker answered: the first one is dropped.
                new File(pendingSave).delete();
                nativeSaveFinished(false, null);
            }
            pendingSave = cachePath;
            Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType(mime);
            intent.putExtra(Intent.EXTRA_TITLE, name);
            try {
                startActivityForResult(intent, REQUEST_SAVE);
            } catch (ActivityNotFoundException e) {
                pendingSave = null;
                new File(cachePath).delete();
                nativeSaveFinished(false, "No app can create documents on this device");
            }
        });
    }

    public void openUrl(final String url) {
        runOnUiThread(() -> {
            try {
                startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url)));
            } catch (Exception e) {
                Log.w(TAG, "couldn't open " + url, e);
            }
        });
    }

    public void setKeyboardVisible(final boolean visible) {
        runOnUiThread(() -> {
            View view = getWindow().getDecorView();
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                WindowInsetsController controller = view.getWindowInsetsController();
                if (controller != null) {
                    if (visible) {
                        controller.show(WindowInsets.Type.ime());
                    } else {
                        controller.hide(WindowInsets.Type.ime());
                    }
                    return;
                }
            }
            InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm == null) {
                return;
            }
            if (visible) {
                imm.showSoftInput(view, InputMethodManager.SHOW_FORCED);
            } else {
                imm.hideSoftInputFromWindow(view.getWindowToken(), 0);
            }
        });
    }

    // ---- Picker answers ------------------------------------------------------------------

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQUEST_OPEN) {
            List<Uri> uris = new ArrayList<>();
            if (resultCode == RESULT_OK && data != null) {
                ClipData clip = data.getClipData();
                if (clip != null) {
                    for (int i = 0; i < clip.getItemCount(); i++) {
                        uris.add(clip.getItemAt(i).getUri());
                    }
                } else if (data.getData() != null) {
                    uris.add(data.getData());
                }
            }
            if (uris.isEmpty()) {
                nativePickCancelled();
            } else {
                deliver(uris);
            }
        } else if (requestCode == REQUEST_SAVE) {
            final String cachePath = pendingSave;
            pendingSave = null;
            if (cachePath == null) {
                return;
            }
            if (resultCode != RESULT_OK || data == null || data.getData() == null) {
                new File(cachePath).delete();
                nativeSaveFinished(false, null);
                return;
            }
            final Uri target = data.getData();
            new Thread(() -> copyToDocument(cachePath, target), "photocraft-save").start();
        }
    }

    private void copyToDocument(String cachePath, Uri target) {
        boolean ok = false;
        String message;
        try (InputStream in = new FileInputStream(cachePath);
             OutputStream out = getContentResolver().openOutputStream(target, "wt")) {
            if (out == null) {
                throw new java.io.IOException("the document provider gave no output stream");
            }
            byte[] buffer = new byte[1 << 16];
            int n;
            while ((n = in.read(buffer)) > 0) {
                out.write(buffer, 0, n);
            }
            ok = true;
            message = displayName(target);
        } catch (Exception e) {
            Log.e(TAG, "save failed", e);
            message = e.getMessage() != null ? e.getMessage() : e.toString();
        }
        new File(cachePath).delete();
        nativeSaveFinished(ok, message);
    }

    // ---- Files opened with or shared to PhotoCraft ------------------------------------------

    private void handleIntent(Intent intent) {
        if (intent == null || intent.getAction() == null) {
            return;
        }
        List<Uri> uris = new ArrayList<>();
        switch (intent.getAction()) {
            case Intent.ACTION_VIEW:
            case Intent.ACTION_EDIT:
                if (intent.getData() != null) {
                    uris.add(intent.getData());
                }
                break;
            case Intent.ACTION_SEND: {
                Uri stream = intent.getParcelableExtra(Intent.EXTRA_STREAM);
                if (stream != null) {
                    uris.add(stream);
                }
                break;
            }
            case Intent.ACTION_SEND_MULTIPLE: {
                ArrayList<Uri> streams = intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM);
                if (streams != null) {
                    uris.addAll(streams);
                }
                break;
            }
            default:
                return;
        }
        // Handled once: a later onCreate after the process restarts must not reopen it.
        intent.setAction(null);
        if (!uris.isEmpty()) {
            deliver(uris);
        }
    }

    /** Opens each document off the UI thread and hands its descriptor to Rust, which reads it. */
    private void deliver(final List<Uri> uris) {
        new Thread(() -> {
            List<String> names = new ArrayList<>();
            List<Integer> fds = new ArrayList<>();
            for (Uri uri : uris) {
                try {
                    ParcelFileDescriptor pfd = getContentResolver().openFileDescriptor(uri, "r");
                    if (pfd == null) {
                        continue;
                    }
                    names.add(displayName(uri));
                    fds.add(pfd.detachFd());
                } catch (Exception e) {
                    Log.e(TAG, "couldn't open " + uri, e);
                }
            }
            if (fds.isEmpty()) {
                nativePickCancelled();
                return;
            }
            int[] raw = new int[fds.size()];
            for (int i = 0; i < raw.length; i++) {
                raw[i] = fds.get(i);
            }
            nativeFilesPicked(names.toArray(new String[0]), raw);
        }, "photocraft-open").start();
    }

    private String displayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                String name = cursor.getString(0);
                if (name != null && !name.isEmpty()) {
                    return name;
                }
            }
        } catch (Exception e) {
            Log.w(TAG, "no display name for " + uri, e);
        }
        String last = uri.getLastPathSegment();
        return last != null ? last : "untitled";
    }

    /** The whole screen for the canvas: status and navigation bars come back with a swipe. */
    @SuppressWarnings("deprecation")
    private void hideSystemBars() {
        View decor = getWindow().getDecorView();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            WindowInsetsController controller = decor.getWindowInsetsController();
            if (controller != null) {
                controller.hide(WindowInsets.Type.systemBars());
                controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            decor.setSystemUiVisibility(View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                    | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN);
        }
    }
}
