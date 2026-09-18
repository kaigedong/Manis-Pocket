import AppKit

struct Selection<Item: Equatable>: Sequence {
    var items: [Item]

    init(items: [Item] = []) {
        self.items = items
    }

    func makeIterator() -> IndexingIterator<[Item]> {
        items.makeIterator()
    }

    var isEmpty: Bool {
        items.isEmpty
    }

    var count: Int {
        items.count
    }

    var first: Item? {
        items.first
    }

    func first(where condition: (Item) -> Bool) -> Item? {
        items.first(where: condition)
    }

    mutating func remove(_ item: Item) {
        items.removeAll { $0 == item }
    }

    mutating func add(_ item: Item) {
        items.append(item)
    }
}
