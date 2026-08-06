package dev.prismaemu.app

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

interface TerminalCallback {
    fun onOutput(text: String)
}

@Composable
fun TerminalView(onDismiss: () -> Unit) {
    var output by remember { mutableStateOf("Prisma Developer Terminal\n") }
    var input by remember { mutableStateOf("") }
    val scrollState = rememberScrollState()
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        scope.launch(Dispatchers.IO) {
            OrchestratorJni.spawnTerminalProcess(object : TerminalCallback {
                override fun onOutput(text: String) {
                    output += text
                }
            })
        }
    }

    // Auto-scroll to bottom when output changes
    LaunchedEffect(output) {
        scrollState.animateScrollTo(scrollState.maxValue)
    }

    Box(modifier = Modifier.fillMaxSize().background(Color.Black)) {
        Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
            Text(
                text = output,
                color = Color.Green,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier
                    .weight(1f)
                    .fillMaxWidth()
                    .verticalScroll(scrollState)
            )

            // Invisible/bottom-aligned TextField for capturing keystrokes
            BasicTextField(
                value = input,
                onValueChange = { newValue ->
                    input = newValue
                    if (newValue.isNotEmpty()) {
                        val lastChar = newValue.last()
                        if (lastChar == '\n') {
                            OrchestratorJni.sendTerminalInput(newValue)
                            input = ""
                        }
                    }
                },
                textStyle = TextStyle(color = Color.Green, fontFamily = FontFamily.Monospace),
                modifier = Modifier.fillMaxWidth().background(Color.DarkGray).padding(8.dp),
                decorationBox = { innerTextField ->
                    if (input.isEmpty()) {
                        Text("Type command...", color = Color.Gray, fontFamily = FontFamily.Monospace)
                    }
                    innerTextField()
                }
            )

            Spacer(modifier = Modifier.height(16.dp))

            Button(
                onClick = onDismiss,
                modifier = Modifier.align(Alignment.End),
                colors = ButtonDefaults.buttonColors(containerColor = Color.DarkGray)
            ) {
                Text("Close Terminal", color = Color.White)
            }
        }
    }
}
