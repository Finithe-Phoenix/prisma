package dev.prismaemu.app

import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            val languagePreferences = remember { LanguagePreferences(this) }
            var languageTag by remember { mutableStateOf(languagePreferences.load()) }
            PrismaLocale(languageTag) {
                PrismaTheme {
                    val store = remember { ContainerStore() }
                    PrismaAppShell(
                        store = store,
                        onRunExe = ::runExecutable,
                        languageTag = languageTag,
                        onLanguageChange = { tag ->
                            languagePreferences.save(tag)
                            languageTag = tag
                        },
                    )
                }
            }
        }
    }

    private fun runExecutable(path: String) {
        val copy = PrismaTechnicalCopies.forTag(LanguagePreferences(this).load())
        lifecycleScope.launch(Dispatchers.IO) {
            try {
                val result = OrchestratorJni.runExecutable(path)
                withContext(Dispatchers.Main) {
                    val message = if (result == 0) {
                        copy.executedSuccessfully
                    } else {
                        "${copy.executionFailed} (code $result)."
                    }
                    Toast.makeText(this@MainActivity, message, Toast.LENGTH_SHORT).show()
                }
            } catch (_: UnsatisfiedLinkError) {
                withContext(Dispatchers.Main) {
                    Toast.makeText(
                        this@MainActivity,
                        copy.arm64Required,
                        Toast.LENGTH_LONG,
                    ).show()
                }
            }
        }
    }
}
