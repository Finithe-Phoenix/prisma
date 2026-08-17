package dev.prismaemu.app

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

interface TerminalCallback {
    fun onOutput(text: String)
}

@Composable
fun TerminalView(onDismiss: () -> Unit) {
    val copy = technicalCopy()
    var output by remember(copy) {
        mutableStateOf("${copy.shellPreview}\n${copy.waitingBridge}\n")
    }
    var input by remember { mutableStateOf("") }
    val scrollState = rememberScrollState()
    val scope = rememberCoroutineScope()

    BackHandler(onBack = onDismiss)

    LaunchedEffect(Unit) {
        scope.launch(Dispatchers.IO) {
            try {
                OrchestratorJni.spawnTerminalProcess(object : TerminalCallback {
                    override fun onOutput(text: String) {
                        scope.launch { output += text }
                    }
                })
            } catch (_: UnsatisfiedLinkError) {
                scope.launch {
                    output += "${copy.bridgeUnavailable}\n"
                }
            }
        }
    }

    LaunchedEffect(output) {
        scrollState.animateScrollTo(scrollState.maxValue)
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(PrismaBackground)
            .statusBarsPadding(),
    ) {
        PrismaTopBar(
            title = copy.developerTerminal,
            subtitle = copy.terminalSubtitle,
            statusColor = PrismaWarning,
            onBack = onDismiss,
        ) {
            TextButton(
                onClick = {
                    output = "${copy.shellPreview}\n"
                },
            ) {
                Text(copy.clear, color = PrismaPrimary)
            }
        }
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
                .background(PrismaInspector.Panel)
                .verticalScroll(scrollState)
                .padding(PrismaSpacing.Lg),
        ) {
            Text(
                text = output,
                color = PrismaTextSecondary,
                style = PrismaTypography.bodyMedium,
                fontFamily = FontFamily.Monospace,
            )
        }
        HorizontalDivider(color = PrismaBorder)
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(PrismaSurface)
                .imePadding()
                .navigationBarsPadding()
                .padding(PrismaSpacing.Lg),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = ">",
                color = PrismaPrimary,
                style = PrismaTypography.bodyMedium,
                fontFamily = FontFamily.Monospace,
            )
            BasicTextField(
                value = input,
                onValueChange = { newValue ->
                    input = newValue
                    if (newValue.endsWith('\n')) {
                        try {
                            OrchestratorJni.sendTerminalInput(newValue)
                        } catch (_: UnsatisfiedLinkError) {
                            output += "> ${newValue.trim()}\n${copy.bridgeUnavailable}\n"
                        }
                        input = ""
                    }
                },
                textStyle = TextStyle(
                    color = PrismaTextPrimary,
                    fontFamily = FontFamily.Monospace,
                ),
                modifier = Modifier
                    .weight(1f)
                    .padding(start = PrismaSpacing.Md),
                decorationBox = { innerTextField ->
                    if (input.isEmpty()) {
                        Text(
                            text = copy.enterCommand,
                            color = PrismaTextMuted,
                            fontFamily = FontFamily.Monospace,
                        )
                    }
                    innerTextField()
                },
            )
        }
    }
}
