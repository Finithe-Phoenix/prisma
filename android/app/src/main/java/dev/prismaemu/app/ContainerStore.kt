package dev.prismaemu.app

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

data class Container(
    val id: String,
    val name: String,
    val winePrefix: String,
    val installedGames: Int,
    val isRunning: Boolean = false
)

data class AppState(
    val containers: List<Container> = listOf(),
    val selectedContainerId: String? = null
)

class ContainerStore : ViewModel() {
    private val _state = MutableStateFlow(
        AppState(
            containers = listOf(
                Container("1", "Default Prefix", "/data/data/dev.prismaemu.app/wine-default", 0),
                Container("2", "Steam Games", "/data/data/dev.prismaemu.app/wine-steam", 2)
            )
        )
    )
    val state: StateFlow<AppState> = _state.asStateFlow()

    fun createContainer(name: String) {
        val newId = (_state.value.containers.size + 1).toString()
        val newPrefix = "/data/data/dev.prismaemu.app/wine-${name.lowercase().replace(" ", "-")}"
        _state.update {
            it.copy(
                containers = it.containers + Container(newId, name, newPrefix, 0)
            )
        }
    }

    fun selectContainer(id: String?) {
        _state.update { it.copy(selectedContainerId = id) }
    }

    fun installExe(containerId: String, exeUri: String) {
        // Mocking the installation process
        _state.update { currentState ->
            val updatedContainers = currentState.containers.map {
                if (it.id == containerId) {
                    it.copy(installedGames = it.installedGames + 1)
                } else {
                    it
                }
            }
            currentState.copy(containers = updatedContainers)
        }
    }
}
