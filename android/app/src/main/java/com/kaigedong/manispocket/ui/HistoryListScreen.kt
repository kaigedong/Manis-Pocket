package com.kaigedong.manispocket.ui

import android.widget.Toast
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.kaigedong.manispocket.ClipboardItem
import com.kaigedong.manispocket.ClipboardService
import com.kaigedong.manispocket.HistoryViewModel
import com.kaigedong.manispocket.LogManager

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HistoryListScreen(viewModel: HistoryViewModel = viewModel()) {
    val context = LocalContext.current
    val items by viewModel.items.collectAsState()
    var searchQuery by remember { mutableStateOf("") }
    var selectedTab by remember { mutableIntStateOf(0) }

    LaunchedEffect(Unit) {
        LogManager.i("ManisPocket", "App started, initializing...")
        viewModel.initialize(context)
        LogManager.i("ManisPocket", "History initialized")

        val clipboardService = ClipboardService(context)
        clipboardService.startPolling { item ->
            LogManager.d("Clipboard", "New clip: ${item.title.take(80)}")
            viewModel.addItem(item)
        }
    }

    Scaffold(
        topBar = {
            if (selectedTab == 0) {
                TopAppBar(
                    title = { Text("Manis Pocket") },
                    colors =
                        TopAppBarDefaults.topAppBarColors(
                            containerColor = MaterialTheme.colorScheme.surface,
                        ),
                )
            }
        },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(
                    icon = { Icon(Icons.Filled.ContentPaste, "History") },
                    label = { Text("History") },
                    selected = selectedTab == 0,
                    onClick = { selectedTab = 0 },
                )
                NavigationBarItem(
                    icon = { Icon(Icons.Filled.Sync, "Sync") },
                    label = { Text("Sync") },
                    selected = selectedTab == 1,
                    onClick = { selectedTab = 1 },
                )
            }
        },
    ) { padding ->
        when (selectedTab) {
            0 ->
                HistoryTab(
                    items = items,
                    searchQuery = searchQuery,
                    onSearchChange = { query ->
                        searchQuery = query
                        if (query.isNotEmpty()) viewModel.search(query)
                    },
                    onCopy = { item ->
                        ClipboardService(context).copyToClipboard(item)
                        LogManager.d("ManisPocket", "Copied: ${item.title.take(80)}")
                    },
                    onDelete = { item ->
                        viewModel.deleteItem(item.id)
                        LogManager.d("ManisPocket", "Deleted: ${item.title.take(80)}")
                    },
                    onTogglePin = { item ->
                        viewModel.togglePin(item.id)
                    },
                    onDownload = { item ->
                        val peerId = item.syncSource ?: ""
                        if (peerId.isNotEmpty()) {
                            viewModel.requestFile(peerId, item.title)
                            Toast.makeText(context, "Downloading...", Toast.LENGTH_SHORT).show()
                        }
                    },
                    modifier = Modifier.padding(padding),
                )
            1 ->
                SyncSettingsScreen(
                    viewModel = viewModel,
                    modifier = Modifier.padding(padding),
                )
        }
    }
}

@Composable
private fun HistoryTab(
    items: List<ClipboardItem>,
    searchQuery: String,
    onSearchChange: (String) -> Unit,
    onCopy: (ClipboardItem) -> Unit,
    onDelete: (ClipboardItem) -> Unit,
    onTogglePin: (ClipboardItem) -> Unit,
    onDownload: (ClipboardItem) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.fillMaxSize()) {
        // Search bar
        OutlinedTextField(
            value = searchQuery,
            onValueChange = onSearchChange,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
            placeholder = { Text("Search clipboard history...") },
            singleLine = true,
            leadingIcon = { Icon(Icons.Filled.Search, "Search") },
            trailingIcon = {
                if (searchQuery.isNotEmpty()) {
                    IconButton(onClick = { onSearchChange("") }) {
                        Icon(Icons.Filled.Clear, "Clear")
                    }
                }
            },
        )

        if (items.isEmpty()) {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center,
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(
                        Icons.Filled.ContentPaste,
                        contentDescription = null,
                        modifier = Modifier.size(48.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f),
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        "Clipboard is empty",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        "Copy something to see it here",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                    )
                }
            }
        } else {
            LazyColumn(modifier = Modifier.fillMaxSize()) {
                items(items, key = { it.id }) { item ->
                    HistoryItemRow(
                        item = item,
                        onCopy = { onCopy(item) },
                        onDelete = { onDelete(item) },
                        onTogglePin = { onTogglePin(item) },
                        onDownload = { onDownload(item) },
                    )
                    HorizontalDivider(modifier = Modifier.padding(horizontal = 16.dp))
                }
            }
        }
    }
}

@Composable
fun HistoryItemRow(
    item: ClipboardItem,
    onCopy: () -> Unit,
    onDelete: () -> Unit,
    onTogglePin: () -> Unit,
    onDownload: () -> Unit,
) {
    val isFile = item.contents.any { it.contentType == "public.file-url" }
    val isRemote = !item.syncSource.isNullOrEmpty()

    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .clickable {
                    if (isFile && isRemote) {
                        onDownload()
                    } else {
                        onCopy()
                    }
                }.padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = item.title.ifEmpty { "(no title)" },
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (isFile) {
                    Icon(
                        Icons.Filled.InsertDriveFile,
                        contentDescription = "File",
                        modifier = Modifier.size(14.dp),
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Spacer(modifier = Modifier.width(4.dp))
                }
                item.application?.let { app ->
                    Text(
                        app,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (isRemote) {
                    if (item.application != null) Text(" · ", style = MaterialTheme.typography.bodySmall)
                    Icon(
                        Icons.Filled.Cloud,
                        contentDescription = "Remote",
                        modifier = Modifier.size(12.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f),
                    )
                }
                if (item.pin != null) {
                    Text(" · ", style = MaterialTheme.typography.bodySmall)
                    Icon(
                        Icons.Filled.PushPin,
                        "Pinned",
                        modifier = Modifier.size(12.dp),
                        tint = MaterialTheme.colorScheme.primary,
                    )
                }
            }
        }

        if (isFile && isRemote) {
            IconButton(onClick = onDownload) {
                Icon(
                    Icons.Filled.CloudDownload,
                    "Download",
                    tint = MaterialTheme.colorScheme.primary,
                )
            }
        }
        IconButton(onClick = onTogglePin) {
            Icon(
                if (item.pin != null) Icons.Filled.PushPin else Icons.Outlined.PushPin,
                contentDescription = "Pin",
                tint =
                    if (item.pin != null) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
            )
        }
        IconButton(onClick = onCopy) {
            Icon(
                Icons.Filled.ContentCopy,
                "Copy",
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        IconButton(onClick = onDelete) {
            Icon(
                Icons.Filled.Delete,
                "Delete",
                tint = MaterialTheme.colorScheme.error,
            )
        }
    }
}
