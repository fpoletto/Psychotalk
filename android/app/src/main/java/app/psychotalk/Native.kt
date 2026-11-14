package app.psychotalk

object Native {
    init { System.loadLibrary("psychotalk_core") }
    external fun nativeDerive(password: ByteArray, salt: ByteArray): ByteArray
    external fun nativeInitialize(directory: String, key: ByteArray): Boolean
    external fun nativeOpen(directory: String, binary: String, key: ByteArray, create: Boolean): Boolean
    external fun nativeCommand(input: String): String
    external fun nativeHeartbeat()
    external fun nativeRevoke()
    external fun nativeClose()
}
