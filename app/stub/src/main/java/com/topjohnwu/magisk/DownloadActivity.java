package com.topjohnwu.magisk;

import static android.R.string.no;
import static android.R.string.ok;
import static android.R.string.yes;
import static com.topjohnwu.magisk.R.string.dling;
import static com.topjohnwu.magisk.R.string.no_internet_msg;
import static com.topjohnwu.magisk.R.string.upgrade_msg;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Context;
import android.content.Intent;
import android.content.res.loader.ResourcesLoader;
import android.content.res.loader.ResourcesProvider;
import android.os.Build;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import android.system.Os;
import android.system.OsConstants;
import android.util.Log;
import android.view.ContextThemeWrapper;

import android.net.Uri;

import com.topjohnwu.magisk.net.Networking;
import com.topjohnwu.magisk.net.Request;
import com.topjohnwu.magisk.utils.APKInstall;

import java.io.ByteArrayInputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.OutputStream;
import java.util.zip.InflaterInputStream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import java.util.zip.ZipOutputStream;

import javax.crypto.Cipher;
import javax.crypto.CipherInputStream;
import javax.crypto.SecretKey;
import javax.crypto.spec.IvParameterSpec;
import javax.crypto.spec.SecretKeySpec;

public class DownloadActivity extends Activity {

    private static final String APP_NAME = "Magisk";

    private Context themed;
    private boolean dynLoad;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        themed = new ContextThemeWrapper(this, android.R.style.Theme_DeviceDefault);

        dynLoad = !getPackageName().equals(BuildConfig.APPLICATION_ID);

        try {
            loadResources();
        } catch (Exception e) {
            error(e);
        }

        ProviderInstaller.install(this);

        if (Networking.checkNetworkStatus(this)) {
            showDialog();
        } else {
            new AlertDialog.Builder(themed)
                    .setCancelable(false)
                    .setTitle(APP_NAME)
                    .setMessage(getString(no_internet_msg))
                    .setNegativeButton(ok, (d, w) -> finish())
                    .show();
        }
    }

    @Override
    public void finish() {
        super.finish();
        Runtime.getRuntime().exit(0);
    }

    private void error(Throwable e) {
        Log.e(getClass().getSimpleName(), Log.getStackTraceString(e));
        finish();
    }

    private Request request(String url) {
        return Networking.get(url).setErrorHandler((conn, e) -> error(e));
    }

    private void showDialog() {
        new AlertDialog.Builder(themed)
                .setCancelable(false)
                .setTitle(APP_NAME)
                .setMessage(getString(upgrade_msg))
                .setPositiveButton(yes, (d, w) -> dlAPK())
                .setNegativeButton(no, (d, w) -> finish())
                .show();
    }

    private void dlAPK() {
        Intent intent = new Intent(Intent.ACTION_VIEW);
        intent.setData(Uri.parse(BuildConfig.APK_URL));
        startActivity(intent);
        finish();
    }

    private void decryptResources(OutputStream out) throws Exception {

        Cipher cipher = Cipher.getInstance("AES/CBC/PKCS5Padding");

        SecretKey key = new SecretKeySpec(Bytes.key(), "AES");
        IvParameterSpec iv = new IvParameterSpec(Bytes.iv());

        cipher.init(Cipher.DECRYPT_MODE, key, iv);

        InflaterInputStream is = new InflaterInputStream(
                new CipherInputStream(
                        new ByteArrayInputStream(Bytes.res()),
                        cipher
                )
        );

        try (InflaterInputStream in = is; OutputStream output = out) {
            APKInstall.transfer(in, output);
        }
    }

    private void loadResources() throws Exception {

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {

            int fd = Os.memfd_create("res", 0);

            try {

                decryptResources(new FileOutputStream(fd));

                Os.lseek(fd, 0, OsConstants.SEEK_SET);

                ResourcesLoader loader = new ResourcesLoader();

                try (ParcelFileDescriptor pfd = ParcelFileDescriptor.dup(fd)) {
                    loader.addProvider(ResourcesProvider.loadFromTable(pfd, null));
                    getResources().addLoaders(loader);
                }

            } finally {
                Os.close(fd);
            }

        } else {

            File res = new File(getCodeCacheDir(), "res.apk");

            try (ZipOutputStream out = new ZipOutputStream(new FileOutputStream(res))) {

                out.putNextEntry(new ZipEntry("AndroidManifest.xml"));

                try (ZipFile stubApk = new ZipFile(getPackageCodePath())) {

                    APKInstall.transfer(
                            stubApk.getInputStream(
                                    stubApk.getEntry("AndroidManifest.xml")
                            ),
                            out
                    );
                }

                out.putNextEntry(new ZipEntry("resources.arsc"));

                decryptResources(out);
            }

            StubApk.addAssetPath(getResources(), res.getPath());
        }
    }
}