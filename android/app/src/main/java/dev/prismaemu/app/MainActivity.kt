package dev.prismaemu.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp

/**
 * F3 launcher entry point. Pure scaffolding today: lists "containers"
 * (the abstraction over a Wine prefix + a translated cache slice).
 * The real container manager — create / launch / delete / per-game
 * config — lands alongside the Wine PE loader (F2) and the dispatcher
 * loop (F1+).
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                PrismaApp()
            }
        }
    }
}

@Composable
fun PrismaApp() {
    Scaffold(
        topBar = {
            TopAppBar(title = { Text("Prisma") })
        }
    ) { padding ->
        ContainerList(modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .padding(16.dp))
    }
}

/** Placeholder data class for the F3 container abstraction. */
data class Container(
    val name: String,
    val winePrefix: String,
    val installedGames: Int,
)

@Composable
fun ContainerList(modifier: Modifier = Modifier) {
    val containers = remember_dummy_containers()
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("Containers", style = MaterialTheme.typography.titleLarge)
        Spacer(modifier = Modifier.height(8.dp))
        if (containers.isEmpty()) {
            Text("No containers yet. Tap + to create one.")
        } else {
            LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items(containers) { container ->
                    ContainerRow(container)
                }
            }
        }
    }
}

@Composable
fun ContainerRow(container: Container) {
    Column {
        Text(container.name, style = MaterialTheme.typography.titleMedium)
        Text(
            "Prefix: ${container.winePrefix} · ${container.installedGames} games",
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

/** Stand-in for a future ContainerStore-backed feed. */
private fun remember_dummy_containers(): List<Container> = listOf(
    Container("Default",   "/data/data/dev.prismaemu.app/wine-default", 0),
)

@Preview(showBackground = true)
@Composable
fun PreviewPrismaApp() {
    MaterialTheme { PrismaApp() }
}

// Tiny shim because Compose's `remember` would require a state holder
// that doesn't exist yet — keep the demo synchronous until the real
// ContainerStore lands.
@Suppress("FunctionName", "NOTHING_TO_INLINE")
private inline fun <T> remember_value(value: T): T = value
