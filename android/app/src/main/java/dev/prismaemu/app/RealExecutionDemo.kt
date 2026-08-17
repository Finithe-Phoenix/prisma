package dev.prismaemu.app

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private data class ProbeCopy(
    val title: String,
    val ready: String,
    val description: String,
    val run: String,
    val running: String,
    val idle: String,
    val real: String,
    val unavailable: String,
    val failed: String,
    val evidence: String,
    val honestNote: String,
)

@Composable
private fun probeCopy(): ProbeCopy = when (LocalPrismaLanguage.current.tag) {
    "es" -> ProbeCopy(
        title = "Ejecución real",
        ready = "PE x86-64 → ARM64",
        description = "Ejecuta un PE32+ real mediante el loader, traductor, backend ARM64, memoria JIT y Session de Prisma.",
        run = "Ejecutar PE real",
        running = "Ejecutando en ARM64 emulado…",
        idle = "Aún no se ha ejecutado",
        real = "EJECUCIÓN CONFIRMADA",
        unavailable = "WORKER NO DISPONIBLE",
        failed = "EJECUCIÓN FALLIDA",
        evidence = "Evidencia producida por el runtime",
        honestNote = "En esta PC, QEMU emula la CPU ARM64. El PE y el JIT de Prisma sí se ejecutan; estas cifras no representan rendimiento de un teléfono.",
    )
    else -> ProbeCopy(
        title = "Real execution",
        ready = "x86-64 PE → ARM64",
        description = "Runs a real PE32+ through Prisma's loader, translator, ARM64 backend, JIT memory, and Session loop.",
        run = "Run real PE",
        running = "Running on emulated ARM64…",
        idle = "Not executed yet",
        real = "EXECUTION CONFIRMED",
        unavailable = "WORKER UNAVAILABLE",
        failed = "EXECUTION FAILED",
        evidence = "Runtime-produced evidence",
        honestNote = "On this PC, QEMU emulates the ARM64 CPU. The PE and Prisma JIT really execute; these numbers are not phone performance results.",
    )
}

@Composable
fun RealExecutionDemo(onDismiss: () -> Unit) {
    val copy = probeCopy()
    val scope = rememberCoroutineScope()
    var report by remember { mutableStateOf<String?>(null) }
    var running by remember { mutableStateOf(false) }
    BackHandler(onBack = onDismiss)

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(PrismaBackground),
    ) {
        PrismaTopBar(
            title = copy.title,
            subtitle = copy.ready,
            statusColor = if (report?.startsWith("REAL|") == true) PrismaSuccess else PrismaPrimary,
            onBack = onDismiss,
        )
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(PrismaSpacing.Lg),
            verticalArrangement = Arrangement.spacedBy(PrismaSpacing.Lg),
        ) {
            item {
                Surface(
                    color = PrismaSurface,
                    shape = RoundedCornerShape(PrismaRadii.Xl),
                    border = BorderStroke(PrismaComponents.Border, PrismaBorder),
                ) {
                    Column(
                        modifier = Modifier.padding(PrismaComponents.CardPadding),
                        verticalArrangement = Arrangement.spacedBy(PrismaSpacing.Lg),
                    ) {
                        Text(
                            text = "PE32+  x86-64   →   SSA   →   ARM64   →   QEMU",
                            color = PrismaPrimary,
                            style = PrismaTypography.labelMedium,
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold,
                        )
                        Text(
                            text = copy.description,
                            color = PrismaTextPrimary,
                            style = PrismaTypography.bodyLarge,
                        )
                        Button(
                            modifier = Modifier.fillMaxWidth(),
                            enabled = !running,
                            colors = ButtonDefaults.buttonColors(
                                containerColor = PrismaPrimary,
                                contentColor = PrismaInk1000,
                            ),
                            onClick = {
                                running = true
                                report = null
                                scope.launch {
                                    report = withContext(Dispatchers.IO) { ExecutionProbeClient.run() }
                                    running = false
                                }
                            },
                        ) {
                            if (running) {
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(PrismaSpacing.Sm),
                                ) {
                                    CircularProgressIndicator(color = PrismaInk1000)
                                    Text(copy.running)
                                }
                            } else {
                                Text(copy.run, fontWeight = FontWeight.Bold)
                            }
                        }
                    }
                }
            }
            item {
                ProbeEvidenceCard(report = report, copy = copy)
            }
            item {
                Surface(
                    color = PrismaWarning.copy(alpha = 0.08f),
                    shape = RoundedCornerShape(PrismaRadii.Lg),
                    border = BorderStroke(PrismaComponents.Border, PrismaWarning.copy(alpha = 0.35f)),
                ) {
                    Text(
                        text = copy.honestNote,
                        modifier = Modifier.padding(PrismaSpacing.Lg),
                        color = PrismaTextSecondary,
                        style = PrismaTypography.bodyMedium,
                    )
                }
            }
        }
    }
}

@Composable
private fun ProbeEvidenceCard(report: String?, copy: ProbeCopy) {
    val status = when {
        report == null -> copy.idle
        report.startsWith("REAL|") -> copy.real
        report.startsWith("UNAVAILABLE|") -> copy.unavailable
        else -> copy.failed
    }
    val statusColor = when {
        report == null -> PrismaTextMuted
        report.startsWith("REAL|") -> PrismaSuccess
        report.startsWith("UNAVAILABLE|") -> PrismaWarning
        else -> PrismaError
    }
    val fields = report
        ?.split('|')
        ?.drop(1)
        ?.mapNotNull { field ->
            val separator = field.indexOf('=')
            if (separator <= 0) null else field.take(separator) to field.drop(separator + 1)
        }
        .orEmpty()

    Surface(
        color = PrismaSurfaceElevated,
        shape = RoundedCornerShape(PrismaRadii.Xl),
        border = BorderStroke(PrismaComponents.Border, statusColor.copy(alpha = 0.45f)),
    ) {
        Column(
            modifier = Modifier.padding(PrismaComponents.CardPadding),
            verticalArrangement = Arrangement.spacedBy(PrismaSpacing.Md),
        ) {
            Text(
                text = copy.evidence.uppercase(),
                color = PrismaTextMuted,
                style = PrismaTypography.labelSmall,
                fontFamily = FontFamily.Monospace,
            )
            Text(
                text = status,
                color = statusColor,
                style = PrismaTypography.titleLarge,
                fontWeight = FontWeight.Bold,
            )
            fields.forEach { (name, value) ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        text = name,
                        color = PrismaTextMuted,
                        style = PrismaTypography.labelMedium,
                        fontFamily = FontFamily.Monospace,
                    )
                    Text(
                        text = value.take(36),
                        color = PrismaTextPrimary,
                        style = PrismaTypography.labelMedium,
                        fontFamily = FontFamily.Monospace,
                    )
                }
            }
        }
    }
}
