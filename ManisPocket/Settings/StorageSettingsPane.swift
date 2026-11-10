import Defaults
import Settings
import SwiftUI

struct StorageSettingsPane: View {
    @Observable
    class ViewModel {
        var saveFiles = false {
            didSet {
                Defaults.withoutPropagation {
                    if saveFiles {
                        Defaults[.enabledPasteboardTypes].formUnion(StorageType.files.types)
                    } else {
                        Defaults[.enabledPasteboardTypes].subtract(StorageType.files.types)
                    }
                }
            }
        }

        var saveImages = false {
            didSet {
                Defaults.withoutPropagation {
                    if saveImages {
                        Defaults[.enabledPasteboardTypes].formUnion(StorageType.images.types)
                    } else {
                        Defaults[.enabledPasteboardTypes].subtract(StorageType.images.types)
                    }
                }
            }
        }

        var saveText = false {
            didSet {
                Defaults.withoutPropagation {
                    if saveText {
                        Defaults[.enabledPasteboardTypes].formUnion(StorageType.text.types)
                    } else {
                        Defaults[.enabledPasteboardTypes].subtract(StorageType.text.types)
                    }
                }
            }
        }

        private var observer: Defaults.Observation?

        init() {
            observer = Defaults.observe(.enabledPasteboardTypes) { change in
                self.saveFiles = change.newValue.isSuperset(of: StorageType.files.types)
                self.saveImages = change.newValue.isSuperset(of: StorageType.images.types)
                self.saveText = change.newValue.isSuperset(of: StorageType.text.types)
            }
        }

        deinit {
            observer?.invalidate()
        }
    }

    @Default(.size) private var size
    @Default(.isUnlimitedHistory) private var isUnlimitedHistory
    @Default(.sortBy) private var sortBy

    @State private var viewModel = ViewModel()
    @State private var storageSize = ""

    private let sizeFormatter: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.minimum = 1
        formatter.maximum = 9999
        return formatter
    }()

    var body: some View {
        Settings.Container(contentWidth: SettingsLayout.contentWidth) {
            Settings.Section(
                bottomDivider: true,
                label: { Text("Save", tableName: "StorageSettings") }
            ) {
                Toggle(
                    isOn: $viewModel.saveFiles,
                    label: { Text("Files", tableName: "StorageSettings") }
                )
                Toggle(
                    isOn: $viewModel.saveImages,
                    label: { Text("Images", tableName: "StorageSettings") }
                )
                Toggle(
                    isOn: $viewModel.saveText,
                    label: { Text("Text", tableName: "StorageSettings") }
                )
                Text("SaveDescription", tableName: "StorageSettings")
                    .controlSize(.small)
                    .foregroundStyle(.gray)
            }

            Settings.Section(label: { Text("Size", tableName: "StorageSettings") }) {
                Defaults.Toggle(key: .isUnlimitedHistory) {
                    Text("UnlimitedHistory", tableName: "StorageSettings")
                }

                HStack {
                    TextField("", value: $size, formatter: sizeFormatter)
                        .frame(width: 80)
                        .help(Text("SizeTooltip", tableName: "StorageSettings"))
                        .disabled(isUnlimitedHistory)
                    Stepper("", value: $size, in: 1 ... 9999)
                        .labelsHidden()
                        .disabled(isUnlimitedHistory)
                    Text(storageSize)
                        .controlSize(.small)
                        .foregroundStyle(.gray)
                        .help(Text("CurrentSizeTooltip", tableName: "StorageSettings"))
                        .onAppear {
                            updateStorageSize()
                        }
                }
                .opacity(isUnlimitedHistory ? 0.5 : 1.0)
            }

            Settings.Section(label: { Text("SortBy", tableName: "StorageSettings") }) {
                Picker("", selection: $sortBy) {
                    ForEach(Sorter.By.allCases) { mode in
                        Text(mode.description)
                    }
                }
                .labelsHidden()
                .frame(width: 160, alignment: .leading)
                .help(Text("SortByTooltip", tableName: "StorageSettings"))
            }
        }
    }

    private func updateStorageSize() {
        let dbPath = History.shared.core.storageSizeBytes(dbPath: History.databasePath)
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        storageSize = formatter.string(fromByteCount: dbPath)
    }
}
