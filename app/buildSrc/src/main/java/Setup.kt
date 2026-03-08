import com.android.build.api.artifact.SingleArtifact
import com.android.build.api.dsl.ApplicationExtension
import com.android.build.api.dsl.CommonExtension
import com.android.build.api.instrumentation.FramesComputationMode.COMPUTE_FRAMES_FOR_INSTRUMENTED_METHODS
import com.android.build.api.instrumentation.InstrumentationScope
import com.android.build.api.variant.AndroidComponentsExtension
import com.android.build.api.variant.ApplicationAndroidComponentsExtension
import org.apache.tools.ant.filters.FixCrLfFilter
import org.gradle.api.Action
import org.gradle.api.JavaVersion
import org.gradle.api.Project
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.StopExecutionException
import org.gradle.api.tasks.Sync
import org.gradle.kotlin.dsl.assign
import org.gradle.kotlin.dsl.exclude
import org.gradle.kotlin.dsl.filter
import org.gradle.kotlin.dsl.get
import org.gradle.kotlin.dsl.register
import org.gradle.kotlin.dsl.withType
import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.tasks.KotlinCompile
import java.io.File
import java.net.URI
import java.security.MessageDigest
import java.util.HexFormat

private fun Project.android(configure: Action<CommonExtension>) =
    extensions.configure("android", configure)

private fun Project.androidApp(configure: Action<ApplicationExtension>) =
    extensions.configure("android", configure)

internal val Project.androidApp: ApplicationExtension
    get() = extensions["android"] as ApplicationExtension

private fun Project.androidComponents(configure: Action<AndroidComponentsExtension<*, *, *>>) =
    extensions.configure(AndroidComponentsExtension::class.java, configure)

private val Project.androidComponents: AndroidComponentsExtension<*, *, *>
    get() = extensions["androidComponents"] as AndroidComponentsExtension<*, *, *>

internal fun Project.androidAppComponents(configure: Action<ApplicationAndroidComponentsExtension>) =
    extensions.configure(ApplicationAndroidComponentsExtension::class.java, configure)

fun Project.setupCommon() {
    android {
        compileSdk {
            version = release(36) {
                minorApiLevel = 1
            }
        }
        buildToolsVersion = "36.1.0"
        ndkPath = "${androidComponents.sdkComponents.sdkDirectory.get().asFile}/ndk/magisk"
        ndkVersion = "29.0.14206865"

        defaultConfig.apply {
            minSdk = 23
        }

        compileOptions.apply {
            sourceCompatibility = JavaVersion.VERSION_21
            targetCompatibility = JavaVersion.VERSION_21
        }

        packaging.apply {
            resources {
                excludes += arrayOf(
                    "/META-INF/*",
                    "/META-INF/androidx/**",
                    "/META-INF/versions/**",
                    "/org/bouncycastle/**",
                    "/org/apache/commons/**",
                    "/kotlin/**",
                    "/kotlinx/**",
                    "/okhttp3/**",
                    "/*.txt",
                    "/*.bin",
                    "/*.json",
                )
            }
        }
    }

    configurations.all {
        exclude("org.jetbrains.kotlin", "kotlin-stdlib-jdk7")
        exclude("org.jetbrains.kotlin", "kotlin-stdlib-jdk8")
    }

    tasks.withType<KotlinCompile> {
        compilerOptions {
            jvmTarget = JvmTarget.JVM_21
        }
    }
}

private fun Project.downloadFile(url: String, checksum: String): File {
    val file = layout.buildDirectory.file(checksum).get().asFile
    if (file.exists()) {
        val md = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { md.update(it.readAllBytes()) }
        val hash = HexFormat.of().formatHex(md.digest())
        if (hash != checksum) {
            file.delete()
        }
    }
    if (!file.exists()) {
        file.parentFile.mkdirs()
        URI(url).toURL().openStream().use { dl ->
            file.outputStream().use {
                dl.copyTo(it)
            }
        }
    }
    return file
}

const val BUSYBOX_DOWNLOAD_URL =
    "https://github.com/topjohnwu/magisk-files/releases/download/files/busybox-1.36.1.1.zip"
const val BUSYBOX_ZIP_CHECKSUM =
    "b4d0551feabaf314e53c79316c980e8f66432e9fb91a69dbbf10a93564b40951"

data class BinaryPackage(
    val url: String,
    val sha256: String
)

val DROPBEAR_PACKAGES = mapOf(
    "arm64-v8a" to BinaryPackage(
        "https://github.com/ribbons/android-dropbear/releases/download/DROPBEAR_2025.89/dropbear-aarch64-linux-android.zip",
        "26f8420f9a1a0e4ac234b0a2d5b62c223f3bf8e2362dd165b12d6d6d44f63844"
    ),
    "armeabi-v7a" to BinaryPackage(
        "https://github.com/ribbons/android-dropbear/releases/download/DROPBEAR_2025.89/dropbear-armv7a-linux-androideabi.zip",
        "0019dfc4b32d63c1392aa264aed2253c1e0c2fb09216f8e2cc269bbfb8bb49b5"
    ),
    "x86_64" to BinaryPackage(
        "https://github.com/ribbons/android-dropbear/releases/download/DROPBEAR_2025.89/dropbear-x86_64-linux-android.zip",
        "8d4f20a774d99df07b1d6ab2cf63b2126986707e17473a2b1c4ee4d49e18d5ad"
    ),
    "x86" to BinaryPackage(
        "https://github.com/ribbons/android-dropbear/releases/download/DROPBEAR_2025.89/dropbear-i686-linux-android.zip",
        "348130870cf13f4baacf0f54ffc48bcff41111de5f26356f95317d518cd70cf2"
    ),
)

private abstract class SyncWithDir : Sync() {
    @get:OutputDirectory
    abstract val outputFolder: DirectoryProperty
}

fun Project.setupCoreLib() {
    setupCommon()

    val abiList = Config.abiList

    androidComponents {
        onVariants { variant ->
            val variantName = variant.name
            val variantCapped = variantName.replaceFirstChar { it.uppercase() }

            val syncLibs = tasks.register("sync${variantCapped}JniLibs", SyncWithDir::class) {
                outputFolder.set(layout.buildDirectory.dir("$variantName/jniLibs"))
                into(outputFolder)

                for (abi in abiList) {
                    into(abi) {
                        from(rootFile("native/out/$abi")) {
                            include("magiskboot", "magiskinit", "magiskpolicy", "magisk", "libinit-ld.so")
                            rename { if (it.endsWith(".so")) it else "lib$it.so" }
                        }
                        val localDropbear = rootFile("tools/dropbear/$abi/dropbear")
                        val localDropbearKey = rootFile("tools/dropbear/$abi/dropbearkey")
                        if (localDropbear.exists()) {
                            from(localDropbear) {
                                rename { "libdropbear.so" }
                            }
                            if (localDropbearKey.exists()) {
                                from(localDropbearKey) {
                                    rename { "libdropbearkey.so" }
                                }
                            }
                        } else {
                            DROPBEAR_PACKAGES[abi]?.let { pkg ->
                                from(zipTree(downloadFile(pkg.url, pkg.sha256))) {
                                    include("dropbear", "dropbearkey")
                                    rename {
                                        when (it) {
                                            "dropbear" -> "libdropbear.so"
                                            "dropbearkey" -> "libdropbearkey.so"
                                            else -> it
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                from(zipTree(downloadFile(BUSYBOX_DOWNLOAD_URL, BUSYBOX_ZIP_CHECKSUM)))
                include(abiList.map { "$it/libbusybox.so" })
                onlyIf {
                    val requiredBinaries = listOf(
                        "magiskboot",
                        "magiskinit",
                        "magiskpolicy",
                        "magisk",
                        "libinit-ld.so"
                    )
                    val missing = abiList.flatMap { abi ->
                        requiredBinaries
                            .filter { !File(rootFile("native/out/$abi"), it).exists() }
                            .map { "$abi/$it" }
                    }
                    if (missing.isNotEmpty()) {
                        throw StopExecutionException(
                            "Please build binaries first! (./build.py binary)\nMissing: ${missing.joinToString(", ")}"
                        )
                    }
                    true
                }
            }

            variant.sources.jniLibs?.let {
                it.addGeneratedSourceDirectory(syncLibs, SyncWithDir::outputFolder)
            }

            val syncResources = tasks.register("sync${variantCapped}Resources", SyncWithDir::class) {
                outputFolder.set(layout.buildDirectory.dir("$variantName/resources"))
                into(outputFolder)

                into("META-INF/com/google/android") {
                    from(rootFile("scripts/update_binary.sh")) {
                        rename { "update-binary" }
                    }
                    from(rootFile("scripts/flash_script.sh")) {
                        rename { "updater-script" }
                    }
                }
            }

            variant.sources.resources?.let {
                it.addGeneratedSourceDirectory(syncResources, SyncWithDir::outputFolder)
            }

            val stubTask = tasks.getByPath(":stub:comment$variantCapped")
            val syncAssets = tasks.register("sync${variantCapped}Assets", SyncWithDir::class) {
                outputFolder.set(layout.buildDirectory.dir("$variantName/assets"))
                into(outputFolder)

                inputs.property("version", Config.version)
                inputs.property("versionCode", Config.versionCode)
                from(rootFile("scripts")) {
                    include("util_functions.sh", "boot_patch.sh", "addon.d.sh",
                        "app_functions.sh", "uninstaller.sh", "module_installer.sh")
                }
                from(rootFile("tools/bootctl"))
                into("chromeos") {
                    from(rootFile("tools/futility"))
                    from(rootFile("tools/keys")) {
                        include("kernel_data_key.vbprivk", "kernel.keyblock")
                    }
                }
                from(stubTask) {
                    include { it.name.endsWith(".apk") }
                    rename { "stub.apk" }
                }
                filesMatching("**/util_functions.sh") {
                    filter {
                        it.replace(
                            "#MAGISK_VERSION_STUB",
                            "MAGISK_VER='${Config.version}'\nMAGISK_VER_CODE=${Config.versionCode}"
                        )
                    }
                    filter<FixCrLfFilter>("eol" to FixCrLfFilter.CrLf.newInstance("lf"))
                }
            }

            variant.sources.assets?.let {
                it.addGeneratedSourceDirectory(syncAssets, SyncWithDir::outputFolder)
            }
        }
    }
}

fun Project.setupAppCommon() {
    setupCommon()

    androidApp {
        signingConfigs {
            val storeFilePath = Config["keyStore"]
            val storePassword = Config["keyStorePass"]
            val keyAlias = Config["keyAlias"]
            val keyPassword = Config["keyPass"]

            create("config") {
                if (!storeFilePath.isNullOrEmpty() && !storePassword.isNullOrEmpty()
                    && !keyAlias.isNullOrEmpty() && !keyPassword.isNullOrEmpty()) {
                    // Config থেকে নাও
                    storeFile = rootFile(storeFilePath)
                    this.storePassword = storePassword
                    this.keyAlias = keyAlias
                    this.keyPassword = keyPassword
                } else {
                    // GitHub secrets থেকে নাও
                    storeFile = rootFile(System.getenv("KEYSTORE_FILE") ?: "key.jks")
                    this.storePassword = System.getenv("KEY_PASSWORD") ?: "magisksu"
                    this.keyAlias = System.getenv("KEY_ALIAS") ?: "magiskpro"
                    this.keyPassword = System.getenv("KEY_PASSWORD") ?: "magisksu"
                }
            }
        }

        defaultConfig {
            targetSdk = 36
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt")
            )
        }

        buildTypes {
            val config = signingConfigs.findByName("config") ?: signingConfigs["debug"]
            debug {
                signingConfig = config
            }
            release {
                signingConfig = config
            }
        }

        lint {
            disable += "MissingTranslation"
            checkReleaseBuilds = false
        }

        dependenciesInfo {
            includeInApk = false
        }

        packaging {
            jniLibs {
                useLegacyPackaging = true
            }
        }
    }

    androidAppComponents {
        onVariants { variant ->
            val variantNameCapped = variant.name.replaceFirstChar { it.uppercase() }

            val commentTask = tasks.register(
                "comment$variantNameCapped",
                AddCommentTask::class.java
            )

            val transformationRequest = variant.artifacts.use(commentTask)
                .wiredWithDirectories(AddCommentTask::apkFolder, AddCommentTask::outFolder)
                .toTransformMany(SingleArtifact.APK)

            val signingConfig = androidApp.buildTypes.getByName(variant.buildType!!).signingConfig

            commentTask.configure {
                this.transformationRequest = transformationRequest
                this.signingConfig = signingConfig
                this.comment = "version=${Config.version}\n" +
                               "versionCode=${Config.versionCode}\n" +
                               "stubVersion=${Config.stubVersion}\n"
                this.outFolder.set(layout.buildDirectory.dir("outputs/apk/${variant.name}"))
            }
        }
    }
}

fun Project.setupMainApk() {
    setupAppCommon()

    androidApp {
        namespace = "com.topjohnwu.magisk"

        defaultConfig {
            applicationId = "com.topjohnwu.magisk"
            vectorDrawables.useSupportLibrary = true
            versionName = Config.version
            versionCode = Config.versionCode
            ndk {
                abiFilters += listOf("armeabi-v7a", "arm64-v8a", "x86", "x86_64", "riscv64")
                debugSymbolLevel = "FULL"
            }
        }
    }

    androidComponents {
        onVariants { variant ->
            variant.instrumentation.apply {
                setAsmFramesComputationMode(COMPUTE_FRAMES_FOR_INSTRUMENTED_METHODS)
                transformClassesWith(
                    DesugarClassVisitorFactory::class.java, InstrumentationScope.ALL) {}
            }
        }
    }
}

const val LSPOSED_DOWNLOAD_URL =
    "https://github.com/LSPosed/LSPosed/releases/download/v1.9.2/LSPosed-v1.9.2-7024-zygisk-release.zip"
const val LSPOSED_CHECKSUM =
    "0ebc6bcb465d1c4b44b7220ab5f0252e6b4eb7fe43da74650476d2798bb29622"

const val SHAMIKO_DOWNLOAD_URL =
    "https://github.com/LSPosed/LSPosed.github.io/releases/download/shamiko-383/Shamiko-v1.2.1-383-release.zip"
const val SHAMIKO_CHECKSUM =
    "93754a038c2d8f0e985bad45c7303b96f70a93d8335060e50146f028d3a9b13f"

fun Project.setupTestApk() {
    setupAppCommon()

    androidComponents {
        onVariants { variant ->
            val variantName = variant.name
            val variantCapped = variantName.replaceFirstChar { it.uppercase() }

            val dlTask = tasks.register("download${variantCapped}Lsposed", SyncWithDir::class) {
                outputFolder.set(layout.buildDirectory.dir("$variantName/lsposed"))
                into(outputFolder)

                from(downloadFile(LSPOSED_DOWNLOAD_URL, LSPOSED_CHECKSUM)) {
                    rename { "lsposed.zip" }
                }
                from(downloadFile(SHAMIKO_DOWNLOAD_URL, SHAMIKO_CHECKSUM)) {
                    rename { "shamiko.zip" }
                }
            }

            variant.sources.assets?.let {
                it.addGeneratedSourceDirectory(dlTask, SyncWithDir::outputFolder)
            }
        }
    }
}
