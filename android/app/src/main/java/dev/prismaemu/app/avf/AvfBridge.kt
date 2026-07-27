package dev.prismaemu.app.avf

import android.content.Context
import android.system.virtualmachine.VirtualMachine
import android.system.virtualmachine.VirtualMachineConfig
import android.system.virtualmachine.VirtualMachineManager
import android.system.virtualmachine.VirtualMachineCallback
import android.system.virtualmachine.VirtualMachineException
import java.io.File
import java.util.concurrent.Executor
import java.util.concurrent.Executors

class AvfBridge(private val context: Context) {

    private val executor: Executor = Executors.newSingleThreadExecutor()
    private var virtualMachine: VirtualMachine? = null

    fun isAvfSupported(): Boolean {
        // Checking if AVF is supported on the device.
        val manager = context.getSystemService(VirtualMachineManager::class.java)
        return manager != null
    }

    fun startHypervisor() {
        try {
            val manager = context.getSystemService(VirtualMachineManager::class.java)
                ?: throw IllegalStateException("VirtualMachineManager not available")

            val builder = VirtualMachineConfig.Builder(context)
                // Using crosvm lightweight hypervisor setup
                .setProtectedVm(false) // Assuming standard VM for paging offloading
            
            val config = builder.build()

            virtualMachine = manager.create("PrismaEmuVM", config)
            virtualMachine?.setCallback(executor, object : VirtualMachineCallback {
                override fun onPayloadStarted(vm: VirtualMachine) {
                    println("VM Payload Started")
                }

                override fun onPayloadReady(vm: VirtualMachine) {
                    println("VM Payload Ready")
                }

                override fun onPayloadFinished(vm: VirtualMachine, exitCode: Int) {
                    println("VM Payload Finished with code: $exitCode")
                }

                override fun onError(vm: VirtualMachine, errorCode: Int, message: String) {
                    println("VM Error: $errorCode, $message")
                }
                
                override fun onStopped(vm: VirtualMachine, reason: Int) {
                    println("VM Stopped, reason: $reason")
                }
            })

            virtualMachine?.run()
        } catch (e: Exception) {
            e.printStackTrace()
            throw RuntimeException("Failed to start AVF Hypervisor", e)
        }
    }

    fun stopHypervisor() {
        try {
            virtualMachine?.close()
            virtualMachine = null
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }
}
