import SwiftUI

struct MultipleSelectionListView<Element: Identifiable, ID: Hashable, Content: View>: View
    where ID == Element.ID
{
    var items: [Element]
    var content: (Element?, Element, Element?, Int) -> Content

    var body: some View {
        LazyVStack(spacing: 0) {
            ForEach(Array(items.enumerated()), id: \.element.id) { index, element in
                let previous = index > 0 ? items[index - 1] : nil
                let next = index < items.count - 1 ? items[index + 1] : nil
                content(previous, element, next, index)
            }
        }
    }
}
