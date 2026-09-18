import SwiftUI

struct IgnoreSettingsPane: View {
    var body: some View {
        TabView {
            IgnoreApplicationsSettingsView()
                .tabItem {
                    Text("ApplicationsTab", tableName: "IgnoreSettings")
                }
            IgnorePasteboardTypesSettingsView()
                .tabItem {
                    Text("PasteboardTypesTab", tableName: "IgnoreSettings")
                }
            IgnoreRegexpsSettingsView()
                .tabItem {
                    Text("RegexpTab", tableName: "IgnoreSettings")
                }
        }
        .padding()
        .frame(width: SettingsLayout.paneWidth)
        .frame(minHeight: 400)
    }
}

#Preview {
    IgnoreSettingsPane()
        .environment(\.locale, .init(identifier: "en"))
}
