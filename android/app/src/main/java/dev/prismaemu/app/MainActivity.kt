package dev.prismaemu.app

import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Build
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.remember

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            PrismaTheme {
                val store = remember { ContainerStore() }
                PrismaAppShell(store) { uriString ->
                    // Launch EXE via JNI on background thread
                    lifecycleScope.launch(Dispatchers.IO) {
                        val result = OrchestratorJni.runExecutable(uriString)
                        withContext(Dispatchers.Main) {
                            if (result == 0) {
                                Toast.makeText(this@MainActivity, "Executed successfully in Rust!", Toast.LENGTH_SHORT).show()
                            } else {
                                Toast.makeText(this@MainActivity, "Execution failed in Rust (Code: $result)", Toast.LENGTH_SHORT).show()
                            }
                        }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PrismaAppShell(store: ContainerStore, onRunExe: (String) -> Unit) {
    val state by store.state.collectAsState()
    var showCreateDialog by remember { mutableStateOf(false) }
    var showSteamImporter by remember { mutableStateOf(false) }
    var showGamepadMapper by remember { mutableStateOf(false) }

    Box(modifier = Modifier.fillMaxSize()) {
        androidx.compose.ui.viewinterop.AndroidView(
            factory = { ctx ->
                android.view.SurfaceView(ctx).apply {
                    holder.addCallback(object : android.view.SurfaceHolder.Callback {
                        override fun surfaceCreated(holder: android.view.SurfaceHolder) {
                            Win32Renderer.surfaceHolder = holder
                        }
                        override fun surfaceChanged(holder: android.view.SurfaceHolder, format: Int, width: Int, height: Int) {}
                        override fun surfaceDestroyed(holder: android.view.SurfaceHolder) {
                            if (Win32Renderer.surfaceHolder == holder) {
                                Win32Renderer.surfaceHolder = null
                            }
                        }
                    })
                }
            },
            modifier = Modifier.fillMaxSize()
        )

        Scaffold(
            topBar = {
            TopAppBar(
                title = { 
                    Text(
                        "PRISMA", 
                        style = MaterialTheme.typography.headlineLarge,
                        fontWeight = FontWeight.Black,
                        color = NeonCyan
                    ) 
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = OLEDBlack.copy(alpha = 0.8f) // Slight glassmorphism
                )
            )
        },
        floatingActionButton = {
            FloatingActionButton(
                onClick = { showCreateDialog = true },
                containerColor = NeonMagenta,
                contentColor = OLEDBlack,
                shape = RoundedCornerShape(16.dp)
            ) {
                Icon(Icons.Default.Add, contentDescription = "New Container")
            }
        },
        containerColor = Color.Transparent
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 16.dp)
        ) {
            Spacer(modifier = Modifier.height(8.dp))
            Button(onClick = { onRunExe("C:\\cube3d.exe") }, modifier = Modifier.fillMaxWidth()) { Text("Launch 3D Game") }
            Spacer(modifier = Modifier.height(8.dp))
            Row(modifier = Modifier.fillMaxWidth()) {
                Button(
                    onClick = { showSteamImporter = true },
                    modifier = Modifier.weight(1f).padding(end = 4.dp),
                    colors = ButtonDefaults.buttonColors(containerColor = NeonMagenta, contentColor = OLEDBlack)
                ) { Text("Steam Import") }
                Button(
                    onClick = { showGamepadMapper = true },
                    modifier = Modifier.weight(1f).padding(start = 4.dp),
                    colors = ButtonDefaults.buttonColors(containerColor = NeonCyan, contentColor = OLEDBlack)
                ) { Text("Gamepad Mapper") }
            }
            Spacer(modifier = Modifier.height(16.dp))
            Text(
                "WINE PREFIXES",
                style = MaterialTheme.typography.titleMedium,
                color = GrayText,
                letterSpacing = 2.dp.value.sp
            )
            Spacer(modifier = Modifier.height(16.dp))

            if (state.containers.isEmpty()) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text("No prefixes found. Tap + to deploy one.", color = GrayText)
                }
            } else {
                LazyColumn(
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                    contentPadding = PaddingValues(bottom = 80.dp)
                ) {
                    items(state.containers) { container ->
                        ContainerCard(container) {
                            store.selectContainer(container.id)
                        }
                    }
                }
            }
        }

        if (showCreateDialog) {
            CreateContainerDialog(
                onDismiss = { showCreateDialog = false },
                onCreate = { name ->
                    store.createContainer(name)
                    showCreateDialog = false
                }
            )
        }

        if (showSteamImporter) {
            SteamLibraryImporter(onDismiss = { showSteamImporter = false })
        }

        state.selectedContainerId?.let { id ->
            val selected = state.containers.find { it.id == id }
            if (selected != null) {
                ContainerActionSheet(
                    container = selected,
                    onDismiss = { store.selectContainer(null) },
                    onInstall = { uri -> 
                        store.installExe(selected.id, uri.toString())
                        store.selectContainer(null)
                        
                        // Copy the .exe from the Content Provider to the Container Prefix
                        lifecycleScope.launch(Dispatchers.IO) {
                            try {
                                val contentResolver = applicationContext.contentResolver
                                val inputStream = contentResolver.openInputStream(uri)
                                if (inputStream != null) {
                                    val prefixDir = java.io.File(applicationContext.filesDir, "wine-${selected.id}/drive_c")
                                    prefixDir.mkdirs()
                                    
                                    val exeFile = java.io.File(prefixDir, "program.exe")
                                    val outputStream = java.io.FileOutputStream(exeFile)
                                    inputStream.copyTo(outputStream)
                                    inputStream.close()
                                    outputStream.close()
                                    
                                    withContext(Dispatchers.Main) {
                                        Toast.makeText(this@MainActivity, "EXE Installed. Launching JIT...", Toast.LENGTH_SHORT).show()
                                    }
                                    
                                    onRunExe(exeFile.absolutePath)
                                }
                            } catch (e: Exception) {
                                e.printStackTrace()
                                withContext(Dispatchers.Main) {
                                    Toast.makeText(this@MainActivity, "Failed to install EXE: ${e.message}", Toast.LENGTH_LONG).show()
                                }
                            }
                        }
                    }
                )
            }
        }
    }

    if (showGamepadMapper) {
        GamepadMapperOverlay(onDismiss = { showGamepadMapper = false })
    }
}
}

@Composable
fun ContainerCard(container: Container, onClick: () -> Unit) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .animateContentSize(),
        shape = RoundedCornerShape(20.dp),
        colors = CardDefaults.cardColors(
            containerColor = DarkSurface
        ),
        elevation = CardDefaults.cardElevation(defaultElevation = 8.dp)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(
                    Brush.horizontalGradient(
                        colors = listOf(
                            DarkSurface,
                            DarkSurfaceVariant
                        )
                    )
                )
                .padding(20.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Box(
                modifier = Modifier
                    .size(48.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(NeonCyan.copy(alpha = 0.1f)),
                contentAlignment = Alignment.Center
            ) {
                Icon(Icons.Default.Build, contentDescription = null, tint = NeonCyan)
            }
            Spacer(modifier = Modifier.width(16.dp))
            Column {
                Text(
                    container.name,
                    style = MaterialTheme.typography.titleLarge,
                    color = WhiteText
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    "Prefix: ${container.winePrefix.takeLast(20)}...",
                    style = MaterialTheme.typography.bodyMedium,
                    color = GrayText
                )
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    "${container.installedGames} Installed Apps",
                    style = MaterialTheme.typography.bodySmall,
                    color = NeonMagenta
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ContainerActionSheet(
    container: Container,
    onDismiss: () -> Unit,
    onInstall: (Uri) -> Unit
) {
    val launcher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenDocument()
    ) { uri: Uri? ->
        if (uri != null) {
            onInstall(uri)
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = DarkSurfaceVariant
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Text(container.name.uppercase(), style = MaterialTheme.typography.headlineLarge, color = NeonCyan)
            Spacer(modifier = Modifier.height(24.dp))
            
            Button(
                onClick = { launcher.launch(arrayOf("application/x-msdownload", "application/octet-stream")) },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(60.dp),
                colors = ButtonDefaults.buttonColors(containerColor = NeonCyan, contentColor = OLEDBlack),
                shape = RoundedCornerShape(12.dp)
            ) {
                Text("RUN .EXE", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
            }
            
            Spacer(modifier = Modifier.height(16.dp))
            
            OutlinedButton(
                onClick = onDismiss,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(60.dp),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = WhiteText),
                border = androidx.compose.foundation.BorderStroke(1.dp, GrayText),
                shape = RoundedCornerShape(12.dp)
            ) {
                Text("CANCEL", style = MaterialTheme.typography.titleMedium)
            }
            
            Spacer(modifier = Modifier.height(32.dp))
        }
    }
}
}

@Composable
fun CreateContainerDialog(onDismiss: () -> Unit, onCreate: (String) -> Unit) {
    var text by remember { mutableStateOf("") }
    
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = DarkSurface,
        title = { Text("New Container", color = WhiteText) },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                label = { Text("Name", color = GrayText) },
                singleLine = true,
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = NeonCyan,
                    unfocusedBorderColor = GrayText,
                    focusedTextColor = WhiteText,
                    unfocusedTextColor = WhiteText
                )
            )
        },
        confirmButton = {
            TextButton(onClick = { if(text.isNotBlank()) onCreate(text) }) {
                Text("CREATE", color = NeonCyan, fontWeight = FontWeight.Bold)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("CANCEL", color = GrayText)
            }
        }
    )
}




@Composable
fun SteamLibraryImporter(onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = DarkSurface,
        title = { Text("Steam Library Importer", color = WhiteText) },
        text = {
            Column {
                Text("Scanning for Steam app manifests...", color = GrayText)
                LinearProgressIndicator(
                    modifier = Modifier.fillMaxWidth().padding(top = 16.dp),
                    color = NeonCyan
                )
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) {
                Text("CANCEL", color = NeonCyan, fontWeight = FontWeight.Bold)
            }
        }
    )
}

@Composable
fun GamepadMapperOverlay(onDismiss: () -> Unit) {
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black.copy(alpha = 0.7f))
            .clickable { onDismiss() },
        contentAlignment = Alignment.Center
    ) {
        Column(
            modifier = Modifier
                .padding(24.dp)
                .background(DarkSurface, RoundedCornerShape(16.dp))
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Text(
                "Gamepad Mapper",
                style = MaterialTheme.typography.headlineMedium,
                color = NeonCyan,
                fontWeight = FontWeight.Bold
            )
            Spacer(modifier = Modifier.height(16.dp))
            Text(
                "Tap on screen areas to map to xinput gamepad actions.",
                color = WhiteText,
                style = MaterialTheme.typography.bodyMedium
            )
            Spacer(modifier = Modifier.height(32.dp))
            Button(
                onClick = onDismiss,
                colors = ButtonDefaults.buttonColors(containerColor = NeonMagenta, contentColor = OLEDBlack)
            ) {
                Text("Close Mapper", fontWeight = FontWeight.Bold)
            }
        }
    }
}
