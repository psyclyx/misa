package org.misa.app

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.LuminanceSource
import com.google.zxing.MultiFormatReader
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import java.util.concurrent.Executors

/**
 * Point a camera at the daemon's code.
 *
 * The daemon prints a QR beside the pairing line, so a phone that had to have the
 * ticket typed into it would be a phone whose pairing story was half a feature.
 * The permission is requested here, at the moment it is needed, because a client
 * that demanded a camera to read a transcript would be asking for the wrong thing.
 */
@Composable
fun QrScanner(onCode: (String) -> Unit, onProblem: (String) -> Unit) {
    val context = LocalContext.current
    var granted by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }
    val request =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
            granted = allowed
            if (!allowed) onProblem("the camera was refused; type or paste the code instead")
        }
    LaunchedEffect(Unit) {
        if (!granted) request.launch(Manifest.permission.CAMERA)
    }
    if (!granted) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Text("waiting for the camera…", style = MaterialTheme.typography.bodyMedium)
        }
        return
    }
    val lifecycleOwner = LocalLifecycleOwner.current
    val executor = remember { Executors.newSingleThreadExecutor() }
    var delivered by remember { mutableStateOf(false) }
    DisposableEffect(Unit) {
        onDispose { executor.shutdown() }
    }
    AndroidView(
        modifier = Modifier.fillMaxSize(),
        factory = { ctx ->
            val preview = PreviewView(ctx)
            val providerFuture = ProcessCameraProvider.getInstance(ctx)
            providerFuture.addListener(
                {
                    val provider = runCatching { providerFuture.get() }.getOrNull() ?: return@addListener
                    val previewUseCase =
                        Preview.Builder().build().also { it.surfaceProvider = preview.surfaceProvider }
                    val analysis =
                        ImageAnalysis.Builder()
                            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                            .build()
                    analysis.setAnalyzer(executor) { image ->
                        val text = runCatching { decode(image) }.getOrNull()
                        image.close()
                        if (text != null && !delivered) {
                            delivered = true
                            preview.post { onCode(text) }
                        }
                    }
                    runCatching {
                        provider.unbindAll()
                        provider.bindToLifecycle(
                            lifecycleOwner,
                            CameraSelector.DEFAULT_BACK_CAMERA,
                            previewUseCase,
                            analysis,
                        )
                    }
                        .onFailure { onProblem("the camera could not start: ${it.message}") }
                },
                ContextCompat.getMainExecutor(ctx),
            )
            preview
        },
    )
}

/** The Y plane of one frame, decoded as a QR. */
private fun decode(image: ImageProxy): String? {
    val plane = image.planes.firstOrNull() ?: return null
    val buffer = plane.buffer
    val rowStride = plane.rowStride
    val data = ByteArray(image.width * image.height)
    for (row in 0 until image.height) {
        buffer.position(row * rowStride)
        buffer.get(data, row * image.width, image.width)
    }
    var source: LuminanceSource =
        PlanarYUVLuminanceSource(data, image.width, image.height, 0, 0, image.width, image.height, false)
    // A phone's camera reports its own rotation, and a code is not readable until
    // the frame is the way up the person is holding it.
    repeat((image.imageInfo.rotationDegrees / 90) % 4) {
        source = source.rotateCounterClockwise()
    }
    val reader = MultiFormatReader()
    reader.setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE)))
    return try {
        reader.decode(BinaryBitmap(HybridBinarizer(source))).text
    } catch (_: NotFoundException) {
        null
    }
}
