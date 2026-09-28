package com.kaigedong.manispocket.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.kaigedong.manispocket.HistoryViewModel

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SyncSettingsScreen(
    viewModel: HistoryViewModel,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    var deviceName by remember {
        val prefs = context.getSharedPreferences("manis_pocket_sync", android.content.Context.MODE_PRIVATE)
        mutableStateOf(prefs.getString("device_name", "Android Device") ?: "Android Device")
    }
    var manualAddress by remember { mutableStateOf("") }
    var showPairingDialog by remember { mutableStateOf(false) }
    var showLogs by remember { mutableStateOf(false) }
    var connectionStatus by remember { mutableStateOf<String?>(null) }

    val syncEnabled by viewModel.syncEnabled.collectAsState()
    val peers by viewModel.peers.collectAsState()
    val pairingRequest by viewModel.pairingRequest.collectAsState()
    val syncError by viewModel.syncError.collectAsState()

    LaunchedEffect(pairingRequest) {
        if (pairingRequest != null) showPairingDialog = true
    }

    LaunchedEffect(syncError) {
        syncError?.let {
            connectionStatus = it
            viewModel.clearError()
        }
    }

    if (showLogs) {
        LogScreen(onBack = { showLogs = false })
        return
    }

    if (showPairingDialog && pairingRequest != null) {
        AlertDialog(
            onDismissRequest = {
                showPairingDialog = false
                viewModel.dismissPairingRequest()
            },
            title = { Text("Pair devices") },
            text = {
                Column {
                    Text("Pairing with \"${pairingRequest!!.displayName}\". Compare this code on both devices.")
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        "PIN: ${pairingRequest!!.pin}",
                        style = MaterialTheme.typography.headlineSmall,
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    viewModel.acceptPairing(pairingRequest!!.peerId, pairingRequest!!.pin)
                    showPairingDialog = false
                    connectionStatus = "Waiting for the other device to confirm..."
                }) { Text("Confirm") }
            },
            dismissButton = {
                TextButton(onClick = {
                    viewModel.rejectPairing(pairingRequest!!.peerId)
                    showPairingDialog = false
                }) { Text("Reject") }
            },
        )
    }

    val paired by viewModel.pairedPeers.collectAsState()

    Scaffold(
        modifier = modifier,
        topBar = {
            TopAppBar(
                title = { Text("Sync Settings") },
                actions = {
                    IconButton(onClick = { showLogs = true }) {
                        Icon(Icons.Filled.BugReport, "Logs")
                    }
                },
            )
        },
    ) { padding ->
        LazyColumn(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(padding)
                    .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // ── Enable ──────────────────────────────────────────
            item {
                Text("Sync", style = MaterialTheme.typography.titleMedium)
                Spacer(modifier = Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text("Enable Clipboard Sync")
                    Switch(
                        checked = syncEnabled,
                        onCheckedChange = { enabled ->
                            if (enabled) {
                                viewModel.startSync(deviceName)
                                connectionStatus = "Starting..."
                            } else {
                                viewModel.stopSync()
                                connectionStatus = null
                            }
                        },
                    )
                }
                connectionStatus?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color =
                            if (it.contains("error", ignoreCase = true)) {
                                MaterialTheme.colorScheme.error
                            } else {
                                MaterialTheme.colorScheme.onSurfaceVariant
                            },
                    )
                }
            }

            // ── Device name ─────────────────────────────────────
            item {
                OutlinedTextField(
                    value = deviceName,
                    onValueChange = { deviceName = it },
                    label = { Text("Device Name") },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    enabled = syncEnabled,
                )
            }

            // ── Paired devices ──────────────────────────────────
            item {
                Text(
                    "Paired Devices",
                    style = MaterialTheme.typography.titleMedium,
                    color =
                        if (syncEnabled) {
                            MaterialTheme.colorScheme.onSurface
                        } else {
                            MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
                        },
                )
            }
            if (paired.isEmpty()) {
                item {
                    Text(
                        "No paired devices",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            } else {
                items(paired, key = { it.peerId }) { peer ->
                    Card(
                        modifier = Modifier.fillMaxWidth(),
                        colors =
                            if (!syncEnabled) {
                                CardDefaults.cardColors(
                                    containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f),
                                )
                            } else {
                                CardDefaults.cardColors()
                            },
                    ) {
                        Row(
                            modifier = Modifier.fillMaxWidth().padding(12.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(peer.displayName, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Box(
                                        modifier =
                                            Modifier
                                                .size(6.dp)
                                                .background(
                                                    if (peer.isConnected) Color(0xFF4CAF50) else Color.Gray,
                                                    CircleShape,
                                                ),
                                    )
                                    Spacer(modifier = Modifier.width(4.dp))
                                    Text(
                                        if (peer.isConnected) "Connected" else "Offline",
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            }
                            if (peer.isAdmin) {
                                TextButton(
                                    onClick = { viewModel.unpair(peer.peerId) },
                                ) {
                                    Text("Unpair", color = MaterialTheme.colorScheme.error)
                                }
                            }
                        }
                    }
                }
            }

            // ── Discovered devices ──────────────────────────────
            item {
                Divider()
                Spacer(modifier = Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        "Discovered Devices",
                        style = MaterialTheme.typography.titleMedium,
                        color =
                            if (syncEnabled) {
                                MaterialTheme.colorScheme.onSurface
                            } else {
                                MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
                            },
                    )
                    IconButton(
                        onClick = { viewModel.refreshDiscovery() },
                        enabled = syncEnabled,
                    ) {
                        Icon(Icons.Filled.Refresh, "Refresh discovery")
                    }
                }
            }
            val discovered = peers.filter { p -> paired.none { it.peerId == p.peerId } }
            if (discovered.isEmpty()) {
                item {
                    Column {
                        Text(
                            "No devices found.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            "Make sure devices are on the same WiFi.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                        )
                    }
                }
            } else {
                items(discovered, key = { it.peerId }) { peer ->
                    Card(
                        modifier = Modifier.fillMaxWidth(),
                        colors =
                            if (!syncEnabled) {
                                CardDefaults.cardColors(
                                    containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f),
                                )
                            } else {
                                CardDefaults.cardColors()
                            },
                    ) {
                        Row(
                            modifier = Modifier.fillMaxWidth().padding(12.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                peer.displayName,
                                modifier = Modifier.weight(1f),
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Button(
                                onClick = { viewModel.requestPairing(peer.peerId) },
                                enabled = syncEnabled,
                            ) { Text("Pair") }
                        }
                    }
                }
            }

            // ── Manual connect ──────────────────────────────────
            item {
                Divider()
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    "Manual Connect",
                    style = MaterialTheme.typography.titleMedium,
                    color =
                        if (syncEnabled) {
                            MaterialTheme.colorScheme.onSurface
                        } else {
                            MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
                        },
                )
            }
            item {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    OutlinedTextField(
                        value = manualAddress,
                        onValueChange = { manualAddress = it },
                        label = { Text("IP:Port") },
                        modifier = Modifier.weight(1f),
                        singleLine = true,
                        enabled = syncEnabled,
                    )
                    Button(
                        onClick = { viewModel.addPeerAddress(manualAddress) },
                        enabled = syncEnabled && manualAddress.isNotEmpty(),
                    ) { Text("Connect") }
                }
            }

            item { Spacer(modifier = Modifier.height(32.dp)) }
        }
    }
}
