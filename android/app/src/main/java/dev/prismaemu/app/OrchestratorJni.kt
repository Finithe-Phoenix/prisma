package dev.prismaemu.app

object OrchestratorJni {
    external fun runExecutable(path: String): Int
    external fun spawnTerminalProcess(callback: Any)
    external fun sendTerminalInput(input: String)
}
