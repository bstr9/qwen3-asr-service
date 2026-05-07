package com.qwen3.asr.typeless

import android.content.Context
import androidx.room.Dao
import androidx.room.Database
import androidx.room.Delete
import androidx.room.Entity
import androidx.room.Insert
import androidx.room.PrimaryKey
import androidx.room.Query
import androidx.room.Room
import androidx.room.RoomDatabase
import androidx.room.migration.Migration
import androidx.sqlite.db.SupportSQLiteDatabase

/**
 * Room database for storing ASR transcription history.
 */

// ---------- Entity ----------

@Entity(tableName = "history")
data class HistoryEntry(
    val text: String,
    val rawText: String,
    val timestamp: Long,
    val durationSecs: Float,
    val mode: String,
    val language: String
) {
    @PrimaryKey(autoGenerate = true)
    var id: Long = 0
}

// ---------- DAO ----------

@Dao
interface HistoryDao {
    @Insert
    suspend fun insert(entry: HistoryEntry): Long

    @Query("SELECT * FROM history ORDER BY timestamp DESC")
    suspend fun getAll(): List<HistoryEntry>

    @Delete
    suspend fun delete(entry: HistoryEntry)
}

// ---------- Database ----------

@Database(entities = [HistoryEntry::class], version = 3, exportSchema = false)
abstract class HistoryDatabase : RoomDatabase() {
    abstract fun historyDao(): HistoryDao

    companion object {
        @Volatile
        private var INSTANCE: HistoryDatabase? = null

        /** Migration from v1 to v2: add `cancelled` column. */
        private val MIGRATION_1_2 = object : Migration(1, 2) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE history ADD COLUMN cancelled INTEGER NOT NULL DEFAULT 0")
            }
        }

        /** Migration from v2 to v3: remove `cancelled` column (recreate table). */
        private val MIGRATION_2_3 = object : Migration(2, 3) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("CREATE TABLE history_tmp (id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, text TEXT NOT NULL, rawText TEXT NOT NULL, timestamp INTEGER NOT NULL, durationSecs REAL NOT NULL, mode TEXT NOT NULL, language TEXT NOT NULL)")
                db.execSQL("INSERT INTO history_tmp (id, text, rawText, timestamp, durationSecs, mode, language) SELECT id, text, rawText, timestamp, durationSecs, mode, language FROM history")
                db.execSQL("DROP TABLE history")
                db.execSQL("ALTER TABLE history_tmp RENAME TO history")
            }
        }

        fun getInstance(context: Context): HistoryDatabase {
            return INSTANCE ?: synchronized(this) {
                INSTANCE ?: Room.databaseBuilder(
                    context.applicationContext,
                    HistoryDatabase::class.java,
                    "typeless_history"
                )
                    .addMigrations(MIGRATION_1_2, MIGRATION_2_3)
                    .fallbackToDestructiveMigration()
                    .build()
                    .also { INSTANCE = it }
            }
        }
    }
}
