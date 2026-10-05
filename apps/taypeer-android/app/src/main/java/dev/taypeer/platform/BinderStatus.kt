package dev.taypeer.platform

import android.os.RemoteException
import dev.taypeer.bridge.AndroidException

/** Private statuses preserve explicit storage outcomes across the two separate Binder ports. */
internal fun code(error: Exception): Int = when (error) {
    is AndroidException.StorageIo -> 1
    is AndroidException.StorageChanged -> 2
    is AndroidException.CommitUncertain -> 3
    is AndroidException.AlreadyExists -> 4
    is AndroidException.InvalidFile -> 5
    is AndroidException.InvalidOptions -> 7
    else -> 6
}
internal fun checked(status: Int) {
    if (status == 0) return
    throw when (status) {
        1 -> AndroidException.StorageIo()
        2 -> AndroidException.StorageChanged()
        3 -> AndroidException.CommitUncertain()
        4 -> AndroidException.AlreadyExists()
        5 -> AndroidException.InvalidFile()
        7 -> AndroidException.InvalidOptions()
        else -> AndroidException.Runtime()
    }
}
internal inline fun <T> remote(block: () -> T): T = try { block() }
    catch (error: AndroidException) { throw error }
    catch (_: Exception) { throw AndroidException.Runtime() }
internal inline fun <T> mutation(block: () -> T): T = try { block() }
    catch (error: AndroidException) { throw error }
    catch (_: RemoteException) { throw AndroidException.CommitUncertain() }
    catch (_: Exception) { throw AndroidException.Runtime() }
internal inline fun status(block: () -> Unit): Int = try { block(); 0 } catch (error: Exception) { code(error) }
