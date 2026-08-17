package dev.prismaemu.app.avf

import android.content.Context

class AvfBridge(private val context: Context) {
    private var virtualMachine: Any? = null

    fun isAvfSupported(): Boolean {
        return runCatching {
            val managerClass = Class.forName(VIRTUAL_MACHINE_MANAGER_CLASS)
            context.getSystemService(managerClass) != null
        }.getOrDefault(false)
    }

    fun startHypervisor() {
        if (!isAvfSupported()) {
            throw UnsupportedOperationException("Android Virtualization Framework is unavailable")
        }

        throw UnsupportedOperationException(
            "AVF startup requires the privileged virtualization SDK integration"
        )
    }

    fun stopHypervisor() {
        val machine = virtualMachine
        virtualMachine = null

        if (machine != null) {
            runCatching {
                machine.javaClass.getMethod("close").invoke(machine)
            }
        }
    }

    private companion object {
        const val VIRTUAL_MACHINE_MANAGER_CLASS =
            "android.system.virtualmachine.VirtualMachineManager"
    }
}
