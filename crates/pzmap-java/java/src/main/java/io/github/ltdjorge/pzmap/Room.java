package io.github.ltdjorge.pzmap;

import java.util.List;

/**
 * A room in a {@link LotHeader}.
 *
 * @param name the room name, such as {@code "kitchen"}
 * @param layer the level the room is on
 * @param rectangles the rectangles that make up the room's floor plan, in world coordinates
 * @param objects the objects placed in the room, in world coordinates
 */
public record Room(String name, int layer, List<Rectangle> rectangles, List<RoomObject> objects) {}
